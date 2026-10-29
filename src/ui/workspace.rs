//! Home workspace, partner card, inspector and repository panes.
use super::*;

pub(super) fn workspace(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let block = panel(
        paint.hint(Message::CoreResponseTitle),
        config,
        paint,
        app.effective_focus() == Pane::Workspace && !app.editing && app.overlay.is_none(),
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let heading = if app.goal.trim().is_empty() {
        Line::styled(paint.hint(Message::CoreRequestInvitation), paint.accent)
    } else {
        Line::from(vec![
            Span::styled(paint.hint(Message::CoreTaskPrefix), paint.accent),
            Span::styled(
                clipped(
                    app.goal.lines().next().unwrap_or(""),
                    content.width.saturating_sub(6) as usize,
                ),
                paint.accent,
            ),
        ])
    };
    let form = app.form().min(8);
    let mut lines = Vec::with_capacity(8);
    lines.push(heading);
    if app.text_density() == store::TextDensity::Comfortable && content.height >= 9 {
        lines.push(Line::from(""));
    }
    if form == 0 {
        lines.extend([
            Line::from(paint.hint(Message::CoreRequestStart)),
            Line::styled(paint.hint(Message::CoreRequestSave), paint.muted),
        ]);
    } else {
        let (signature, action, label) = crate::domain::FIRST_ACTIONS[form];
        lines.extend([
            Line::from(format!(
                "{} / {}: {signature}",
                crate::domain::FORMS[form].name,
                crate::domain::FORMS[form].element
            )),
            Line::styled(
                match action {
                    Some(command) => format!("/{command} {label}"),
                    None => label.to_owned(),
                },
                paint.accent,
            ),
        ]);
    }
    lines.extend([
        Line::from(""),
        Line::styled(paint.hint(Message::CoreRequestContext), paint.muted),
        Line::styled(paint.hint(Message::CoreRequestTasks), paint.muted),
    ]);
    if app
        .drafts
        .iter()
        .any(|draft| draft.field != EditorField::Request)
    {
        lines.push(Line::styled(
            paint.hint(Message::CoreRetainedTaskFact),
            paint.muted,
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        content,
    );
}

pub(super) fn compact_companion(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    config: &Config,
    paint: &Paint,
    artwork: &mut Artwork,
    show_image: bool,
) {
    let focused = app.effective_focus() == Pane::Companion;
    let block = panel(" PARTNER ", config, paint, focused);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(frame, row(content, 0, 1), &app.lead_label(), paint.accent);
    text(
        frame,
        row(content, 1, 1),
        FORMS[app.form()].element,
        paint.muted,
    );
    let image = companion_portrait(area);
    if show_image && app.nickname_editor.is_none() {
        artwork.render(app.form(), image, frame.buffer_mut());
    }
    let status = if let Some(todo) = todo_line(app) {
        Cow::Owned(todo)
    } else if app.criteria.is_empty() {
        Cow::Borrowed("No criteria yet")
    } else {
        let met = app
            .criteria
            .iter()
            .filter(|criterion| criterion.met)
            .count();
        Cow::Owned(format!("Objectives {met}/{}", app.criteria.len()))
    };
    text(
        frame,
        row(content, content.height.saturating_sub(2), 1),
        &clipped(&status, content.width as usize),
        paint.muted,
    );
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        if focused && app.last_carry_over().is_some() {
            paint.hint(Message::CoreCarryOverControl)
        } else if focused {
            paint.hint(Message::CoreFormControl)
        } else {
            "/forms | /companion"
        },
        paint.accent,
    );
}

pub(super) fn companion(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
    artwork: &mut Artwork,
    show_image: bool,
) {
    let block = panel(
        " Companion ",
        config,
        paint,
        app.effective_focus() == Pane::Companion || app.overlay == Some(Overlay::Companion),
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let form = &FORMS[app.form()];
    frame.render_widget(
        Paragraph::new(app.lead_label())
            .style(paint.accent)
            .alignment(Alignment::Center),
        row(content, 0, 1),
    );
    frame.render_widget(
        Paragraph::new(if app.last_carry_over().is_some() {
            "/forms /images  n Nickname  c Carry-over"
        } else {
            "/forms | /images / n Nickname"
        })
        .style(paint.accent)
        .alignment(Alignment::Center),
        row(content, 1, 1),
    );
    let image = portrait(area);
    if show_image && app.nickname_editor.is_none() {
        artwork.render(app.form(), image, frame.buffer_mut());
    }
    let details = row(
        content,
        2 + image.height,
        content.height.saturating_sub(2 + image.height),
    );
    let mut lines = Vec::new();
    if app.last_carry_over().is_some() {
        lines.extend([
            Line::styled("C=criteria met/total P=committed pins", paint.muted),
            Line::styled("H=open hypotheses F=open findings", paint.muted),
        ]);
    }
    lines.extend([
        Line::styled(form.element, paint.muted),
        Line::from(form.role),
    ]);
    if let Some(todo) = todo_line(app) {
        lines.push(Line::styled(todo, paint.accent));
    }
    lines.extend([
        Line::styled(artwork.status(app.form()), paint.muted),
        Line::styled("/party Roster | /lead Front a partner", paint.accent),
    ]);
    scroll_text(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app,
        ScrollSurface::Companion,
    );
}

pub(super) fn inspect(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
    artwork: &Artwork,
    graphics_status: &str,
) {
    let drawer = app.overlay == Some(Overlay::Inspector);
    let block = panel(
        " Task and storage inspector ",
        config,
        paint,
        drawer || app.effective_focus() == Pane::Inspector,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let project = std::path::Path::new(&app.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&app.cwd);
    let project = display_identity(project);
    let task = if app.task_id > 0 {
        format!("#{}", app.task_id)
    } else {
        "New / not committed".to_owned()
    };
    let value = if drawer {
        let path = config.assets_dir.to_string_lossy();
        let safe_path: String = path.chars().filter(|c| !c.is_control()).collect();
        let updated = if app.updated_at > 0 {
            ago(app.updated_at)
        } else {
            "Not committed yet".to_owned()
        };
        let quota = app.policy.quota_bytes;
        let sampled = app.usage_bytes;
        let retention = app.policy.retention_days;
        format!(
            "TASK\n{task} / {}\nUpdated: {updated}\nCriteria: {}/{} met\nRetained drafts: {}\nComplete means user-reviewed criteria, not verified command evidence.\n\nPROJECT DIRECTORY\n{}\n\nSTORAGE\n{}\nRevision {} / committed {}\nDirectory: {}\n{}\nRecovery: /storage-retry; --export-data FILE exports readable local records, not provider history.\nQuota: {quota} bytes. Last sample: {sampled} live SQLite, retained output and committed reservations; not physical database size.\nRetention: archived at least {retention} days; explicit purge only, no automatic eviction.\nSecret detection is best-effort. Volatile editor text may remain unmasked.\nArchive is reversible. Deletion affects selected local records and owned artifacts, not repository files, exports or provider sessions. Not forensic secure erasure.\n\nEXECUTION\n/provider opens connection selection. /run inspects local command evidence; /execute reviews a prepared command. /handoff opens portable handoff. Cancellation does not roll back files; approval is not a sandbox.\n\nARTWORK\n{}\n{}\n{}\n\nUp/Down scroll / Esc back",
            if app.complete { "Complete" } else { "Open" },
            app.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            app.criteria.len(),
            app.drafts.len(),
            app.cwd.escape_debug(),
            app.storage_label(),
            app.revision,
            app.saved_revision,
            config.data_dir.to_string_lossy().escape_debug(),
            match &app.storage {
                crate::app::StorageState::Failed(error) => error.as_str(),
                _ => "",
            }
            .escape_debug(),
            artwork.status(app.form()),
            graphics_status,
            safe_path
        )
    } else {
        format!(
            "PROJECT\n{project}\n\nTASK\n{task} / {}\nCriteria: {}/{} met\nRetained drafts: {}\n\nSTORAGE\n{}\n{}\n\n/provider Requests | /run Local checks | /handoff Handoff\nUp/Down scroll",
            if app.complete { "Complete" } else { "Open" },
            app.criteria
                .iter()
                .filter(|criterion| criterion.met)
                .count(),
            app.criteria.len(),
            app.drafts.len(),
            app.storage_label(),
            match &app.storage {
                crate::app::StorageState::Failed(error) => error.as_str(),
                _ => "",
            }
            .escape_debug(),
        )
    };
    scroll_text(
        frame,
        content,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app,
        ScrollSurface::Inspector,
    );
}

pub(super) fn repository(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    let area = sheet(area, 106, area.height);
    frame.render_widget(Clear, area);
    let block = panel(" Repository / read-only evidence ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    text(frame, row(content, 0, 1), &app.repo_status, paint.accent);
    let details = row(content, 1, content.height.saturating_sub(1));
    let value = if app.repo_text.is_empty() {
        if app.repo_loading {
            "Reading repository on a bounded worker; task editing remains available after Esc."
        } else {
            "No repository evidence available. A non-Git directory still supports durable tasks."
        }
    } else {
        &app.repo_text
    };
    let lines: Vec<_> = app
        .repo_error
        .lines()
        .map(|line| Line::styled(line, paint.muted))
        .chain(value.lines().map(Line::from))
        .collect();
    let max = selection_details(
        frame,
        details,
        Paragraph::new(lines)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        app.scroll_for(ScrollSurface::Preview),
    );
    app.set_scroll_viewport(ScrollSurface::Preview, max, details.height);
}
