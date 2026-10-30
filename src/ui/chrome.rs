//! Header, footer, choice lines and confirmations around the surfaces.
use super::*;

/// Choice/control lines mark the selected action as `[Label]`; paint that
/// segment as a button so the current choice stands out from its alternatives.
pub(super) fn choice_text<'a>(value: &'a str, style: Style, selected: Style) -> Text<'a> {
    let lines = value.split('\n').map(|line| {
        let mut spans = Vec::new();
        let mut rest = line;
        while let Some(start) = rest.find('[') {
            let Some(end) = rest[start..].find(']').map(|end| start + end) else {
                break;
            };
            if !rest[start + 1..end]
                .chars()
                .next()
                .is_some_and(char::is_uppercase)
            {
                spans.push(Span::styled(&rest[..=end], style));
                rest = &rest[end + 1..];
                continue;
            }
            if start > 0 {
                spans.push(Span::styled(&rest[..start], style));
            }
            spans.push(Span::styled(&rest[start..=end], selected));
            rest = &rest[end + 1..];
        }
        if !rest.is_empty() {
            spans.push(Span::styled(rest, style));
        }
        Line::from(spans)
    });
    Text::from(lines.collect::<Vec<_>>()).style(style)
}

pub(super) fn choices(frame: &mut Frame, area: Rect, value: &str, style: Style, paint: &Paint) {
    frame.render_widget(
        Paragraph::new(choice_text(value, style, paint.selected)).wrap(Wrap { trim: false }),
        area,
    );
}

/// Provider, model and effort at a glance; `/verbose` adds observation caveats.
pub(super) fn selection_text(state: &crate::app::planning::State) -> String {
    if VERBOSE.get() {
        state.selection_label()
    } else {
        format!("{} · {}", state.agent.label(), state.selection.summary())
    }
}

pub(super) fn activity_marker(frame: u8, config: &Config, motion_enabled: bool) -> &'static str {
    if !motion_enabled || config.no_animation {
        "*"
    } else if config.ascii {
        ["|", "/", "-", "\\"][usize::from(frame) % 4]
    } else {
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][usize::from(frame) % 10]
    }
}

pub(super) fn header(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    let paint = &paint.on_canvas();
    let project = std::path::Path::new(&app.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&app.cwd);
    let project = display_identity(project);
    let mode = paint.hint(if app.lead().auto_form {
        Message::CoreEvolutionAuto
    } else {
        Message::CoreEvolutionManual
    });
    let brand = if area.width >= 90 { " EEVEE " } else { "" };
    let usable_width = usize::from(area.width).saturating_sub(brand.len());
    {
        // An unconnected default route must not read as a ready connection;
        // Enter (or /connect) opens setup.
        let short = crate::providers::capabilities::route(app.planning.agent).short;
        let route_label = match app.agents.readiness(app.planning.agent.index()) {
            crate::app::agents::Readiness::Setup
            | crate::app::agents::Readiness::MissingCli
            | crate::app::agents::Readiness::SignedOut
                if area.width >= 90 =>
            {
                format!("{short} (not connected)")
            }
            crate::app::agents::Readiness::Setup
            | crate::app::agents::Readiness::MissingCli
            | crate::app::agents::Readiness::SignedOut => format!("{short} (setup)"),
            crate::app::agents::Readiness::Blocked => format!("{short} (attention)"),
            _ => short.to_owned(),
        };
        let route = route_label.as_str();
        // "default/default" says nothing twice; show the effort only when chosen
        // or when a specific model makes the pairing meaningful.
        let effort_text = match (
            app.planning.selection.model.as_deref(),
            app.planning.selection.effort.as_deref(),
        ) {
            (None, None) => String::new(),
            (_, effort) => format!("/{}", clipped(effort.unwrap_or("default"), 8)),
        };
        let effort = Span::styled(effort_text, paint.muted);
        let model_name = app.planning.selection.model.as_deref().unwrap_or("default");
        let model_hint_width = Span::raw(model_name).width().min(20);
        let identity_budget = usable_width
            .saturating_sub(Span::raw(route).width() + effort.width() + model_hint_width + 6);
        let suffix = format!(" ({}) [{mode}]", FORMS[app.form()].name);
        let name = App::partner_label(app.lead());
        let identity = format!(
            "{}{suffix}",
            clipped(
                &name,
                identity_budget.saturating_sub(Span::raw(suffix.as_str()).width())
            )
        );
        let spare = usable_width.saturating_sub(
            Span::raw(identity.as_str()).width()
                + Span::raw(route).width()
                + effort.width()
                + model_hint_width
                + 6,
        );
        let context = if spare >= 7 {
            format!("{identity} · {} · {route} · ", clipped(&project, spare - 3))
        } else {
            format!("{identity} · {route} · ")
        };
        let context = Span::styled(context, paint.muted);
        let model_width = usable_width.saturating_sub(context.width() + effort.width());
        let model = Span::styled(
            clipped(
                app.planning.selection.model.as_deref().unwrap_or("default"),
                model_width,
            ),
            paint.accent,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(brand, paint.selected),
                context,
                model,
                effort,
            ])),
            row(area, 0, 1),
        );
    }
    let waiting = app.approval_count() > 0;
    let activity = if waiting {
        paint.hint(Message::CoreWaitingForUser)
    } else if app.planning.cancelling {
        "Cancelling provider | /response Status"
    } else if let Some(preview) = app.planning.preview.as_ref() {
        if preview.blockers.is_empty() {
            "Request prepared | /approvals Details"
        } else {
            "Request blocked | /approvals Details"
        }
    } else if app.planning.preference_error().is_some() {
        "Model preference not saved | /storage-retry"
    } else if let Some(activity) = app.planning.activity_label() {
        activity
    } else if app.planning.outcome_unknown() {
        if area.width < 90 {
            "Outcome unknown | /response"
        } else {
            "Provider outcome unknown | /response Inspect"
        }
    } else {
        app.execution.activity_label()
    };
    let policy = match app.planning.explicit_operation() {
        Some(crate::providers::Operation::Plan) => paint.hint(Message::CorePolicyReadonly),
        Some(crate::providers::Operation::Review) => paint.hint(Message::CorePolicyReview),
        // A standing grant must stay visible even when the chip has no room.
        _ if app.auto_approve_session => paint.hint(Message::AutoApproveSessionPolicy),
        _ if app.planning.auto_approve => paint.hint(Message::AutoApprovePolicy),
        _ => "",
    };
    let marker = if !waiting && app.planning.activity_label().is_some() {
        format!(
            "{} ",
            activity_marker(app.activity_frame, config, app.motion_enabled())
        )
    } else {
        String::new()
    };
    let mut status = format!(
        "{policy}{}{} · {marker}{activity}",
        if policy.is_empty() { "" } else { " · " },
        app.storage_label(),
    );
    // Without the companion pane, keep the todo step and next form visible
    // here; activity, approvals and cancellation take the row instead.
    if let Some(todo) = compact_todo(app, frame.area().width) {
        status = format!("{status} · {todo}");
    }
    if app.has_queued_send() {
        status = format!("{status} · Next Send queued | /queue-cancel");
    }
    if app.mcp.effects_unknown() {
        status = format!("MCP effects unknown · manual review | {status}");
    }
    let line = row(area, 1, 1);
    text(
        frame,
        line,
        &clipped(&status, area.width as usize),
        paint.muted,
    );
    // Capability chip (EV03A): lowest header priority, so it appears only in
    // space the activity/storage text leaves free and never clips it.
    let operation = if app.planning.busy() {
        app.planning.operation
    } else {
        app.planning
            .explicit_operation()
            .unwrap_or(crate::providers::Operation::Implement)
    };
    let chip = format!(
        "[{}] ",
        paint.hint(match operation {
            crate::providers::Operation::Plan | crate::providers::Operation::Review =>
                Message::CoreCapabilityReadonly,
            crate::providers::Operation::Implement => Message::CoreCapabilityReviewedCoding,
        })
    );
    let chip = if app.auto_approve_session && operation == crate::providers::Operation::Implement {
        format!("[{}] ", paint.hint(Message::AutoApproveSessionChip))
    } else if app.planning.auto_approve && operation == crate::providers::Operation::Implement {
        format!("[{}] ", paint.hint(Message::AutoApproveChip))
    } else {
        chip
    };
    let (chip_width, used) = (
        Span::raw(chip.as_str()).width() as u16,
        Span::raw(status.as_str()).width() as u16,
    );
    if used + 2 + chip_width <= line.width {
        text(
            frame,
            Rect::new(line.right() - chip_width, line.y, chip_width, 1),
            &chip,
            paint.accent,
        );
        // Usage meter (XC09): left of the capability chip, only in spare room.
        if let Some(meter) = app.meter_chip(area.width) {
            let meter = format!("{meter} ");
            let width = Span::raw(meter.as_str()).width() as u16;
            if used + 3 + chip_width + width <= line.width {
                text(
                    frame,
                    Rect::new(line.right() - chip_width - width, line.y, width, 1),
                    &meter,
                    paint.muted,
                );
            }
        }
    }
}

pub(super) fn suggestion_line<'a>(app: &'a App, paint: &Paint) -> Option<Line<'a>> {
    // Automatic mode chooses the form itself at Send; an advisory strip would
    // compete with that choice. The picker still marks the recommendation.
    if app.planning.request.is_some() || app.lead().auto_form {
        return None;
    }
    app.visible_suggestion().map(|suggestion| {
        let mut spans = vec![
            Span::raw("Suggested: "),
            Span::raw(FORMS[suggestion.form.min(FORMS.len() - 1)].name),
            Span::raw(" — "),
            Span::raw(suggestion.reason.as_str()),
        ];
        if let Some(partner) = app.suggested_partner() {
            spans.push(Span::raw(format!(
                " · {} ({}) is free",
                App::partner_label(partner),
                FORMS[partner.form].name
            )));
        }
        Line::from(spans).style(paint.accent)
    })
}

/// Descriptive local evidence only; rendering never evaluates or selects a form.
pub(super) fn suggestion_strip(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) -> Rect {
    if app.planning.has_current_file_output() && !app.carry_over_explicit() {
        return area;
    }
    if let Some(carry_over) = app.carry_over() {
        // The precomputed ASCII summary fits the minimum workspace width.
        // Give this transient record the existing strip slot, not a second row.
        frame.render_widget(
            Paragraph::new(carry_over.summary.as_str()).style(paint.accent),
            row(area, 0, 1),
        );
        return row(area, 1, area.height.saturating_sub(1));
    }
    // A repeated request Eevee could learn is the one actionable offer here.
    if let Some(proposal) = app.move_proposal_visible() {
        let line = Line::from(vec![
            Span::styled("Eevee wants to learn ", paint.accent),
            Span::styled(proposal.name, paint.accent.add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(
                    " · you sent this {} times · /learn-move · /learn-dismiss",
                    proposal.sources.len()
                ),
                paint.accent,
            ),
        ]);
        let paragraph = Paragraph::new(line).wrap(Wrap { trim: false });
        let height = paragraph
            .line_count(area.width.max(1))
            .min(usize::from(area.height.saturating_sub(6))) as u16;
        frame.render_widget(paragraph, row(area, 0, height));
        return row(area, height, area.height.saturating_sub(height));
    }
    // A deliberate saved item takes the advisory slot; carry-over stays inspectable.
    if app.held.target.is_some() && !app.held.dismissed {
        return area;
    }
    let Some(line) = suggestion_line(app, paint) else {
        return area;
    };
    let paragraph = Paragraph::new(line).wrap(Wrap { trim: false });
    // A completed response needs Request, metadata, notice, two navigation
    // rows and at least one body row. Advisory wrapping must not consume it.
    let height = paragraph
        .line_count(area.width.max(1))
        .min(usize::from(area.height.saturating_sub(6))) as u16;
    frame.render_widget(paragraph, row(area, 0, height));
    row(area, height, area.height.saturating_sub(height))
}

/// Whether the status row shows the todo line itself (narrow layouts only).
pub(super) fn compact_todo(app: &App, width: u16) -> Option<String> {
    let companion = !app.linear && width >= 90;
    let busy = app.approval_count() > 0
        || app.planning.cancelling
        || app.planning.preview.is_some()
        || app.planning.activity_label().is_some()
        || app.planning.outcome_unknown();
    if companion || busy {
        return None;
    }
    let todo = todo_line(app)?;
    let used = Span::raw(app.storage_label()).width()
        + Span::raw(app.execution.activity_label()).width()
        + 6;
    (used + Span::raw(todo.as_str()).width() <= usize::from(width)).then_some(todo)
}

/// The todo line for the composer title when the narrow status row has no room.
pub(super) fn composer_todo(app: &App, width: u16) -> Option<String> {
    if (!app.linear && width >= 90) || compact_todo(app, width).is_some() {
        return None;
    }
    let busy = app.approval_count() > 0
        || app.planning.cancelling
        || app.planning.activity_label().is_some();
    (!busy).then(|| todo_line(app)).flatten()
}

/// The todo step that steers automatic evolution, or a waiting proposal.
pub(super) fn todo_line(app: &App) -> Option<String> {
    if let Some((position, form)) = app.todo_guide() {
        return Some(match form {
            // Lead with the destination so clipping keeps it visible.
            Some(form) => format!("→ {} · Todo {position}", FORMS[form].name),
            None => format!("Todo {position}"),
        });
    }
    app.workflows
        .progress
        .as_ref()
        .filter(|progress| {
            progress.revision > 0 && progress.adopted_revision != Some(progress.revision)
        })
        .map(|_| "Todo proposal waiting · /todo".into())
}

pub(super) fn footer(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) {
    let value = if app.quit_after_save {
        paint.hint(Message::CoreSavingExit)
    } else if app.confirmation.is_some() {
        paint.hint(Message::CoreConfirmationControl)
    } else if app.reset_after_save {
        paint.hint(Message::CoreSavingReset)
    } else if app.transitioning {
        paint.hint(Message::CoreApplyingTask)
    } else if app.command_popup.is_some() {
        paint.hint(Message::CoreCommandPickerControl)
    } else if app.composer_menu.is_some() && app.overlay.is_none() {
        paint.hint(Message::CoreFilePickerControl)
    } else if app.approval_count() > 0 && app.approval_shortcut_available() {
        if app.own_approval_pending()
            && (app.composer_is_request() || app.effective_focus() == Pane::Workspace)
        {
            paint.hint(Message::CorePendingRequestControl)
        } else {
            paint.hint(Message::CorePendingReviewControl)
        }
    } else if app.editing && app.overlay.is_none() {
        if app.composer_is_request() && app.planning.busy() {
            paint.hint(Message::CoreRequestBusyControl)
        } else if app.composer_is_request() && app.response_on_home() {
            paint.hint(Message::CoreResponseRequestControl)
        } else if app.composer_is_request() {
            paint.hint(Message::CoreRequestControl)
        } else {
            paint.hint(Message::CoreTaskFactControl)
        }
    } else {
        match app.overlay {
            Some(Overlay::Forms) => paint.hint(Message::CoreFormsControl),
            Some(Overlay::Criteria) if area.width < 70 => {
                paint.hint(Message::CoreCriteriaControlCompact)
            }
            Some(Overlay::Criteria) => paint.hint(Message::CoreCriteriaControl),
            Some(Overlay::Tasks) if area.width < 70 => paint.hint(Message::CoreTasksControlCompact),
            Some(Overlay::Tasks) => paint.hint(Message::CoreTasksControl),
            Some(Overlay::Planning)
                if app.planning.request.is_some() || app.planning.preview.is_some() =>
            {
                paint.hint(Message::CoreApprovalsControl)
            }
            Some(Overlay::Planning) if app.planning.activity_label().is_some() => {
                paint.hint(Message::CoreRequestActiveControl)
            }
            Some(Overlay::Sources) if app.sources.editor.is_some() => {
                paint.hint(Message::CoreSourceFilterControl)
            }
            Some(
                Overlay::Execution
                | Overlay::Agents
                | Overlay::Planning
                | Overlay::Handoff
                | Overlay::Workflows
                | Overlay::Memory
                | Overlay::Held
                | Overlay::Updates
                | Overlay::PartyBoard,
            ) => paint.hint(Message::CoreOverlayControl),
            Some(_) => paint.hint(Message::CoreOverlayScrollControl),
            None => paint.hint(Message::CoreHomeControl),
        }
    };
    let queued_notice = app
        .approval_response_queued()
        .then(|| paint.hint(Message::CoreResponseQueued));
    let notice = if !app.notice.is_empty() {
        Some(
            app.motion_notice()
                .or_else(|| app.density_notice())
                .or_else(|| app.formation_notice())
                .or_else(|| {
                    [
                        (
                            Message::AutoApproveEnabled,
                            Message::AutoApproveEnabledNotice,
                        ),
                        (Message::AutoApproveDisabled, Message::AutoApproveDisabled),
                    ]
                    .into_iter()
                    .find_map(|(source, display)| (app.notice == source.text()).then_some(display))
                })
                .map_or_else(
                    || brief(&app.notice),
                    |message| Cow::Borrowed(paint.hint(message)),
                ),
        )
    } else {
        queued_notice.map(Cow::Borrowed)
    };
    if let Some(notice) = notice {
        text(
            frame,
            row(area, 0, 1),
            &clipped(&notice, area.width as usize),
            paint.accent,
        );
        text(frame, row(area, 1, 1), value, paint.base);
    } else if app.scroll() > 0 && !app.editing {
        text(
            frame,
            area,
            &paint.format(
                Template::CoreDetailPosition,
                &[("value", value), ("line", &(app.scroll() + 1).to_string())],
            ),
            paint.base,
        );
    } else {
        // Hints sit on the bottom row so the notice row above stays free.
        let lines = value.lines().count().max(1) as u16;
        let top = area.height.saturating_sub(lines);
        text(frame, row(area, top, area.height - top), value, paint.base);
    }
}

pub(super) fn confirmation(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    let Some(confirm) = app.confirmation.as_ref() else {
        return;
    };
    if confirm.kind == ConfirmationKind::DeleteTasks {
        delete_confirmation(frame, area, app, config, paint);
        return;
    }
    let area = if area.width < 30 || area.height < 10 {
        area
    } else {
        sheet(area, 62, 10)
    };
    frame.render_widget(Clear, area);
    let block = panel(
        match confirm.kind {
            ConfirmationKind::Complete => " Confirm task completion ",
            ConfirmationKind::Reopen(_) => " Reopen completed task ",
            ConfirmationKind::DiscardDraft => " Review draft discard ",
            ConfirmationKind::Quit => " Confirm exit ",
            ConfirmationKind::ResetTask => " Reset local task ",
            ConfirmationKind::ReplaceWithInit => " Prepare /init request ",
            ConfirmationKind::ReturnToEevee => " Return to Eevee ",
            ConfirmationKind::DeleteTasks => unreachable!("separate deletion preview"),
        },
        config,
        paint,
        true,
    );
    let content = if area.width < 8 || area.height < 5 {
        area
    } else {
        block.inner(area)
    };
    frame.render_widget(block, area);
    let (question, keep, discard) = match confirm.kind {
        ConfirmationKind::DiscardDraft => (
            "Discard this field's draft? Applied task data is kept.",
            "Keep editing",
            "Discard draft",
        ),
        ConfirmationKind::Complete => (
            "Mark Complete based on your checklist review? This is not verified command evidence.",
            "Stay",
            "Complete task",
        ),
        ConfirmationKind::Reopen(_) => (
            "Reopen this completed task before editing its goal or criteria?",
            "Stay",
            "Reopen and edit",
        ),
        ConfirmationKind::Quit if matches!(app.storage, crate::app::StorageState::Failed(_)) => (
            "Storage failed. Unsaved edits may be lost; accepted saves may finish. Stay, then r retries.",
            "Stay",
            "Exit despite save failure",
        ),
        ConfirmationKind::Quit if app.sources.store_failed => (
            "Context storage failed. The retained operation is not saved. Stay, fix storage, then v and s retry.",
            "Stay",
            "Exit despite unsaved context",
        ),
        ConfirmationKind::Quit if app.moves.learning.is_some() => (
            "Exit and discard the unsaved move name/learning choice? Task drafts are saved; only committed moves survive.",
            "Keep editing",
            "Discard move draft and quit",
        ),
        ConfirmationKind::Quit => (
            "Exit? Save the applied task and retained drafts before closing.",
            "Stay",
            "Save task/drafts and quit",
        ),
        ConfirmationKind::ResetTask => (
            "Fresh local task? Save old task + draft in /tasks. No files, history or credentials erased. Active work must finish/cancel first.",
            "Stay",
            "Save and reset",
        ),
        ConfirmationKind::ReplaceWithInit => (
            "Replace Request draft with /init proposal? Other drafts stay. Select Espeon / Plan, clear write paths. No send/write; review then Run.",
            "Keep draft",
            "Replace with /init",
        ),
        ConfirmationKind::ReturnToEevee => (
            "Return to intake? Cancel provider work and wait for it to finish first. Drafts and evidence stay; cancellation does not roll back effects.",
            "Stay",
            "Cancel and wait",
        ),
        ConfirmationKind::DeleteTasks => unreachable!("separate deletion preview"),
    };
    text(frame, row(content, 0, 3), question, paint.surface);
    text(
        frame,
        row(content, 3, 1),
        &format!("[n] {keep}"),
        if !confirm.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, 4, 1),
        &format!("[y] {discard}"),
        if confirm.destructive {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, 6, 2),
        paint.hint(Message::CoreConfirmationControl),
        paint.muted,
    );
    app.confirmation_reviewed = content.height >= 8
        && content.width >= 44
        && Paragraph::new(question)
            .wrap(Wrap { trim: false })
            .line_count(content.width.max(1))
            <= 3;
}

pub(super) fn delete_confirmation(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    config: &Config,
    paint: &Paint,
) {
    let area = sheet(area, 78, 26);
    frame.render_widget(Clear, area);
    let block = panel(" Review local task deletion ", config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let question = format!(
        "Delete {} listed tasks, drafts, evidence, task memories and screenshot bytes? Shared memories, project files, exports and provider sessions stay. Not secure erasure. Page {}/{} (Home/End).",
        app.delete_targets.len(),
        app.delete_page + 1,
        app.deletion_pages()
    );
    text(frame, row(content, 0, 4), &question, paint.surface);
    let details = row(content, 4, content.height.saturating_sub(8));
    enum Item<'a> {
        Task(&'a crate::store::Snapshot),
        Excerpt(i64, &'a str),
        Screenshot(&'a crate::workspace::context::screenshot::Capture, bool),
        Run(&'a str, bool),
        Provider(&'a str, crate::providers::Agent),
        Memory(&'a crate::workspace::memory::Record),
    }
    let lines: Vec<_> = app
        .delete_preview
        .iter()
        .flat_map(|target| {
            std::iter::once(Item::Task(&target.task))
                .chain(
                    target
                        .excerpts
                        .iter()
                        .map(|(id, path)| Item::Excerpt(*id, path)),
                )
                .chain(
                    target
                        .screenshots
                        .iter()
                        .map(|(capture, available)| Item::Screenshot(capture, *available)),
                )
                .chain(
                    target
                        .runs
                        .iter()
                        .map(|(id, available)| Item::Run(id, *available)),
                )
                .chain(
                    target
                        .provider_records
                        .iter()
                        .map(|(id, agent)| Item::Provider(id, *agent)),
                )
                .chain(target.memories.iter().map(Item::Memory))
        })
        .skip(app.delete_page * crate::app::DELETE_PREVIEW_PAGE)
        .take(crate::app::DELETE_PREVIEW_PAGE)
        .map(|item| {
            Line::from(match item {
                Item::Task(task) => format!(
                    "#{} {}",
                    task.id,
                    clipped(
                        task.goal.lines().next().unwrap_or("Untitled task"),
                        details.width.saturating_sub(24) as usize
                    )
                ),
                Item::Excerpt(id, path) => format!(
                    "  excerpt #{id} {}",
                    clipped(path, details.width.saturating_sub(32) as usize)
                ),
                Item::Screenshot(capture, available) => format!(
                    "  screenshot partner #{} {:?} / {} / {} B {}",
                    capture.partner,
                    capture.path,
                    capture.format.label(),
                    if available { capture.bytes } else { 0 },
                    if available { "retained" } else { "unavailable" },
                ),
                Item::Run(id, available) => format!(
                    "  run {id} / output {}",
                    if available { "retained" } else { "unavailable" }
                ),
                Item::Provider(id, agent) => {
                    format!("  provider {} / observation {id}", agent.label())
                }
                Item::Memory(record) => format!(
                    "  task memory #{} {}",
                    record.id,
                    clipped(
                        &record.proposal.content,
                        details.width.saturating_sub(24) as usize
                    )
                ),
            })
        })
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
    let selected = app
        .confirmation
        .is_some_and(|confirmation| confirmation.destructive);
    text(
        frame,
        row(content, content.height.saturating_sub(4), 1),
        "[n] Stay / keep all data",
        if !selected {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, content.height.saturating_sub(3), 1),
        "[y] Delete the listed local tasks",
        if selected {
            paint.selected
        } else {
            paint.surface
        },
    );
    text(
        frame,
        row(content, content.height.saturating_sub(2), 2),
        paint.hint(Message::CoreDeleteControl),
        paint.muted,
    );
    app.confirmation_reviewed = content.height >= 8
        && content.width >= 44
        && Paragraph::new(question)
            .wrap(Wrap { trim: false })
            .line_count(content.width.max(1))
            <= 4;
}
