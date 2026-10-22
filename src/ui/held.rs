//! Local preference chrome. Rendering never selects context or changes the receipt.
use ratatui::{Frame, layout::Rect, widgets::Clear};

use super::{Paint, panel, row, text};
use crate::{
    app::{
        App,
        held::{Notice, Status},
    },
    config::Config,
    domain::FORMS,
    i18n::{Message, Template},
    workspace::held::Target,
};

fn status(status: Status) -> Message {
    match status {
        Status::Empty => Message::HeldEmpty,
        Status::Saved => Message::HeldSaved,
        Status::Proposed => Message::HeldProposed,
        Status::Included => Message::HeldIncluded,
        Status::Unavailable => Message::HeldUnavailable,
        Status::Dismissed => Message::HeldDismissed,
        Status::Saving => Message::HeldSaving,
        Status::Failed => Message::HeldFailed,
    }
}

fn notice(notice: &Notice) -> Message {
    match notice {
        Notice::ChooseFile => Message::HeldChooseFile,
        Notice::ChooseMemory => Message::HeldChooseMemory,
        Notice::Saved => Message::HeldNoticeSaved,
        Notice::Waiting => Message::HeldWaiting,
        Notice::Busy => Message::HeldBusy,
        Notice::Unavailable => Message::HeldNoticeUnavailable,
        Notice::ReviewFile => Message::HeldReviewFile,
        Notice::Accepted => Message::HeldAccepted,
        Notice::Dismissed => Message::HeldNoticeDismissed,
        Notice::NoProposal => Message::HeldNoProposal,
        Notice::QueueFull => Message::HeldQueueFull,
        Notice::TooSmall => Message::HeldTooSmall,
    }
}

pub(super) fn target_label(target: Option<&Target>, paint: &Paint) -> String {
    match target {
        Some(Target::File { path }) => path.clone(),
        Some(Target::Memory { id, revision }) => format!("#{id} @ {revision}"),
        None => paint.hint(Message::HeldNoTarget).to_owned(),
    }
}

fn chip_status(status: Status) -> Message {
    match status {
        Status::Empty => Message::HeldChipEmpty,
        Status::Saved => Message::HeldChipSaved,
        Status::Proposed => Message::HeldChipProposed,
        Status::Included => Message::HeldChipIncluded,
        Status::Unavailable => Message::HeldChipUnavailable,
        Status::Dismissed => Message::HeldChipDismissed,
        Status::Saving => Message::HeldChipSaving,
        Status::Failed => Message::HeldChipFailed,
    }
}

pub(super) fn strip(frame: &mut Frame, area: Rect, app: &App, paint: &Paint) -> Rect {
    // Consent, file output and the literal composer always win over optional chrome.
    if app.overlay.is_some()
        || app.approval_count() > 0
        || app.sources.confirmation.is_some()
        || app.planning.has_current_file_output()
        || app.write_owner().is_some()
        || area.height < 6
        || app.held.target.is_none()
        || app.held.dismissed
    {
        return area;
    }
    let target = match &app.held.target {
        Some(Target::File { path }) => format!("{path:?}"),
        _ => app.held_target_label(),
    };
    let label = paint.format(
        Template::HeldStrip,
        &[
            ("status", paint.hint(chip_status(app.held_status()))),
            ("target", &target),
        ],
    );
    let mut identity = row(area, 0, 1);
    identity.width = identity.width.saturating_sub(6);
    text(
        frame,
        identity,
        &super::clipped(&label, usize::from(identity.width)),
        paint.muted,
    );
    text(
        frame,
        Rect::new(identity.right(), area.y, 6, 1),
        " /held",
        paint.accent,
    );
    row(area, 1, area.height.saturating_sub(1))
}

pub(super) fn render(frame: &mut Frame, area: Rect, app: &App, config: &Config, paint: &Paint) {
    frame.render_widget(Clear, area);
    if area.width < 60 || area.height < 18 {
        text(frame, area, paint.hint(Message::HeldTooSmall), paint.accent);
        return;
    }
    let area = super::centered(area, 110, area.height.min(24));
    let block = panel(paint.hint(Message::HeldPanel), config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let scope = paint.format(
        Template::HeldScope,
        &[
            ("task", &app.task_id.to_string()),
            ("owner", &app.lead_label()),
            ("form", FORMS[app.form()].name),
        ],
    );
    let project = paint.format(
        Template::HeldProject,
        &[("project", &super::display_identity(&app.cwd))],
    );
    let target = paint.format(
        Template::HeldTarget,
        &[("target", &app.held_target_label())],
    );
    let mut lines = vec![
        scope,
        project,
        paint.hint(status(app.held_status())).to_owned(),
        target,
    ];
    lines.push(paint.hint(Message::HeldReceipt).to_owned());
    if let Some(receipt) = &app.held.receipt {
        lines.push(paint.format(
            Template::HeldScope,
            &[
                ("task", &receipt.task.to_string()),
                ("owner", &receipt.owner.name),
                ("form", FORMS[receipt.owner.form.min(FORMS.len() - 1)].name),
            ],
        ));
        lines.push(paint.format(
            Template::HeldTarget,
            &[("target", &target_label(receipt.target.as_ref(), paint))],
        ));
        lines.push(
            paint
                .hint(if receipt.included {
                    Message::HeldReceiptIncluded
                } else {
                    Message::HeldReceiptExcluded
                })
                .to_owned(),
        );
    } else {
        lines.push(paint.hint(Message::HeldNoReceipt).to_owned());
    }
    if let Some(value) = &app.held.notice {
        lines.push(paint.hint(notice(value)).to_owned());
    }
    lines.push(paint.hint(Message::HeldPolicy).to_owned());
    // One explicit row per fact keeps all actions visible at the minimum size.
    // Long identities are clipped signposts, never consent or copied source text.
    let actions = [
        Message::HeldActionsFile,
        Message::HeldActionsMemory,
        Message::HeldActionsAccept,
        Message::HeldActionsClear,
        Message::HeldActionsOrdinary,
    ];
    let body_height = content.height.saturating_sub(actions.len() as u16);
    for (index, line) in lines.iter().take(body_height as usize).enumerate() {
        text(
            frame,
            row(content, index as u16, 1),
            &super::clipped(line, content.width as usize),
            paint.surface,
        );
    }
    for (index, action) in actions.iter().enumerate() {
        text(
            frame,
            row(content, body_height + index as u16, 1),
            paint.hint(*action),
            paint.accent,
        );
    }
}
