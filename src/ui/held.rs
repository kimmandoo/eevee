//! Local preference chrome. Rendering never selects context or changes the receipt.
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, panel, row, text};
use crate::{
    app::{
        App,
        held::{Notice, Status},
    },
    config::Config,
    domain::FORMS,
    messages::{Message, Template},
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
    if let Some(review) = &app.held.renewal {
        review.reviewed.set(false);
    }
    if area.width < 60 || area.height < 18 {
        text(frame, area, paint.hint(Message::HeldTooSmall), paint.accent);
        return;
    }
    if app.held.renewal.is_some() || app.held.inspection.is_some() {
        render_reference_review(frame, area, app, config, paint);
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

fn render_reference_review(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    config: &Config,
    paint: &Paint,
) {
    let area = super::centered(area, 110, area.height);
    let block = panel(paint.hint(Message::HeldRenewalPanel), config, paint, true);
    let content = block.inner(area);
    frame.render_widget(block, area);
    let body = row(content, 0, content.height.saturating_sub(2));
    let footer = row(content, content.height.saturating_sub(2), 2);
    let (detail, scroll) = if let Some(review) = &app.held.renewal {
        let detail = review.details.get_or_init(|| {
            let scope = paint.format(Template::HeldScope, &[
                ("task", &review.task.to_string()), ("owner", &review.intent.owner.name),
                ("form", FORMS[review.intent.owner.form].name),
            ]);
            let old = target_label(Some(&review.intent.expected), paint);
            let new = target_label(Some(&review.intent.proposed), paint);
            // Match Sources' exact escaped-byte presentation, never interpreting controls.
            let old_content = review.old_content.as_deref().map(|text| format!("{text:?}"))
                .unwrap_or_else(|| paint.hint(Message::HeldRenewalOldUnavailable).into());
            let old_disclosure = if matches!(review.intent.expected, Target::File { .. }) {
                paint.hint(Message::HeldRenewalFileDisclosure)
            } else { "" };
            let file_identity = review.intent.file_proof.as_ref().map(|proof| {
                paint.format(Template::HeldRenewalFileIdentity, &[
                    ("root", &proof.excerpt.root),
                    ("start", &proof.excerpt.start_line.to_string()),
                    ("end", &proof.excerpt.end_line.to_string()),
                    ("digest", &format!("{:02x?}", proof.excerpt.source_sha256)),
                ])
            }).unwrap_or_default();
            format!("{scope}\n{}\n{}\n{old}\n{old_disclosure}\n{old_content}\n{}\n{}\n{new}\n{file_identity}\n{:?}\n{}\n{}",
                super::display_identity(&app.cwd),
                paint.hint(Message::HeldRenewalOld),
                paint.hint(Message::HeldRenewalContentEscaped),
                paint.hint(Message::HeldRenewalNew),
                review.intent.file_proof.as_ref().map(|proof| proof.excerpt.text.as_str())
                    .unwrap_or(&review.new_content),
                paint.hint(Message::HeldRenewalPolicy),
                paint.hint(Message::HeldRenewalFrozen))
        });
        let geometry = (area.width, area.height);
        let max = if review.geometry.replace(geometry) != geometry {
            review.reviewed.set(false);
            let max = Paragraph::new(detail.as_str())
                .wrap(Wrap { trim: false })
                .line_count(body.width.max(1))
                .saturating_sub(body.height as usize)
                .min(u16::MAX as usize) as u16;
            review.scroll_max.set(max);
            max
        } else {
            review.scroll_max.get()
        };
        let scroll = review.scroll.min(max);
        review.reviewed.set(
            scroll == max
                && body.height >= 4
                && app.overlay == Some(crate::app::Overlay::Held)
                && app.command_popup.is_none()
                && app.confirmation.is_none(),
        );
        super::choices(
            frame,
            footer,
            paint.hint(if !app.held_renewal_can_confirm() {
                Message::HeldRenewalWaiting
            } else if review.reviewed.get() {
                Message::HeldRenewalConfirm
            } else {
                Message::HeldRenewalScroll
            }),
            paint.accent,
            paint,
        );
        (detail, scroll)
    } else {
        let scope = app.held.inspection.as_ref().unwrap();
        let detail = scope.details.get_or_init(|| {
            let identity = paint.format(
                Template::HeldScope,
                &[
                    ("task", &app.task_id.to_string()),
                    ("owner", &scope.owner.name),
                    ("form", FORMS[scope.owner.form].name),
                ],
            );
            let readiness = match scope.readiness {
                crate::app::held::Readiness::NotConfigured => Message::HeldInspectionNotConfigured,
                crate::app::held::Readiness::Available => Message::HeldInspectionAvailable,
                crate::app::held::Readiness::Unavailable => Message::HeldInspectionUnavailable,
                crate::app::held::Readiness::Unknown => Message::HeldInspectionUnknown,
            };
            format!(
                "{identity}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                super::display_identity(&app.cwd),
                paint.hint(Message::HeldInspectionPolicy),
                target_label(scope.target.as_ref(), paint),
                paint.hint(readiness),
                paint.hint(Message::HeldRenewalContentEscaped),
                scope
                    .content
                    .as_deref()
                    .map(|text| format!("{text:?}"))
                    .unwrap_or_else(|| paint.hint(Message::HeldRenewalOldUnavailable).into()),
                paint.hint(Message::HeldRenewalFrozen)
            )
        });
        let max = Paragraph::new(detail.as_str())
            .wrap(Wrap { trim: false })
            .line_count(body.width.max(1))
            .saturating_sub(body.height as usize)
            .min(u16::MAX as usize) as u16;
        text(
            frame,
            footer,
            paint.hint(Message::HeldInspectionReturn),
            paint.accent,
        );
        (detail, scope.scroll.min(max))
    };
    frame.render_widget(
        Paragraph::new(detail.as_str())
            .wrap(Wrap { trim: false })
            .style(paint.surface)
            .scroll((scroll, 0)),
        body,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{Overlay, held::Review},
        workspace::held::Renewal,
    };
    use std::cell::{Cell, OnceCell};

    pub(super) fn draw(app: &App, width: u16, height: u16) {
        let config = Config {
            native_transport: crate::providers::NativeTransport::Auto,
            retry: Default::default(),
            launch_session: Default::default(),
            ascii: true,
            no_animation: true,
            animation_override: Some(false),
            theme: crate::config::Theme::Dark,
            color: crate::config::ColorMode::Mono,
            images: crate::config::ImageMode::Off,
            assets_dir: Default::default(),
            layout: crate::config::LayoutMode::Linear,
            layout_explicit: true,
            data_dir: Default::default(),
            storage_policy: Default::default(),
            context_exclusions: vec![],
            capture_capacity: 1024,
        };
        let paint = Paint::new(&config, &FORMS[0], 0);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), app, &config, &paint))
            .unwrap();
    }

    #[test]
    fn full_reference_scope_must_be_seen_and_hidden_or_undersize_review_cannot_approve() {
        for (width, height) in [(60, 18), (120, 36)] {
            let mut app = App::new("project".into(), width, height);
            app.overlay = Some(Overlay::Held);
            let new_content = "Literal bounded retained content 한글 界e\u{301}\n".repeat(80);
            let old_path = format!("retained/{}/old.rs", "한글/".repeat(12));
            app.held.renewal = Some(Review {
                task: 7,
                intent: Renewal {
                    expected: Target::File {
                        path: old_path.clone(),
                    },
                    proposed: Target::Memory {
                        id: 21,
                        revision: 8,
                    },
                    owner: app.attribution(),
                    task_revision: 4,
                    exclusions: vec![],
                    file_proof: None,
                },
                old_content: None,
                new_content: new_content.clone(),
                scroll: 0,
                scroll_max: Cell::new(0),
                reviewed: Cell::new(false),
                geometry: Cell::new((0, 0)),
                details: OnceCell::new(),
            });
            draw(&app, width, height);
            let review = app.held.renewal.as_mut().unwrap();
            assert!(!review.reviewed.get());
            assert!(review.scroll_max.get() > 0);
            assert!(review.details.get().unwrap().contains(&old_path));
            assert!(
                review
                    .details
                    .get()
                    .unwrap()
                    .contains(&format!("{new_content:?}"))
            );
            review.scroll = review.scroll_max.get();
            draw(&app, width, height);
            assert!(app.held.renewal.as_ref().unwrap().reviewed.get());
            app.overlay = None;
            draw(&app, width, height);
            assert!(!app.held.renewal.as_ref().unwrap().reviewed.get());
            app.overlay = Some(Overlay::Held);
            draw(&app, 40, 10);
            assert!(!app.held.renewal.as_ref().unwrap().reviewed.get());
        }
    }
}

#[cfg(test)]
mod control_content_tests {
    use super::*;
    use crate::app::{
        Overlay,
        held::{Inspection, Readiness, Review},
    };
    use crate::workspace::{
        context::Excerpt,
        held::{FileProof, Renewal},
    };
    use std::cell::{Cell, OnceCell};

    #[test]
    fn renewal_and_scoped_inspection_escape_controls_without_changing_retained_bytes() {
        let raw = "literal CRLF\r\nESC:\u{1b}[31m bidi:\u{202e} 한글";
        let mut app = App::new("/fixture".into(), 60, 18);
        app.overlay = Some(Overlay::Held);
        let proof = FileProof::new(
            Excerpt {
                origin_run: None,
                id: 0,
                task_id: 7,
                root: "/fixture".into(),
                path: "new.rs".into(),
                start_line: 1,
                end_line: 2,
                source_sha256: [19; 32],
                text: raw.into(),
                available: true,
            },
            vec![],
        );
        app.held.renewal = Some(Review {
            task: 7,
            intent: Renewal {
                expected: Target::File {
                    path: "old.rs".into(),
                },
                proposed: Target::File {
                    path: "new.rs".into(),
                },
                owner: app.attribution(),
                task_revision: 4,
                exclusions: vec![],
                file_proof: Some(proof.clone()),
            },
            old_content: Some(raw.into()),
            new_content: String::new(),
            scroll: 0,
            scroll_max: Cell::new(0),
            reviewed: Cell::new(false),
            geometry: Cell::new((0, 0)),
            details: OnceCell::new(),
        });
        super::tests::draw(&app, 60, 18);
        let review = app.held.renewal.as_ref().unwrap();
        let details = review.details.get().unwrap();
        assert_eq!(details.matches(&format!("{raw:?}")).count(), 2);
        assert!(
            !details
                .chars()
                .any(|ch| matches!(ch, '\r' | '\u{1b}' | '\u{202e}'))
        );
        assert_eq!(review.intent.file_proof.as_ref(), Some(&proof));
        assert_eq!(review.old_content.as_deref(), Some(raw));
        app.held.renewal = None;
        app.held.inspection = Some(Inspection {
            owner: crate::store::Attribution {
                form: app.form(),
                ..crate::store::Attribution::of(app.lead())
            },
            target: Some(Target::File {
                path: "old.rs".into(),
            }),
            content: Some(raw.into()),
            return_to: Some(Overlay::Formations),
            scroll: 0,
            details: OnceCell::new(),
            readiness: Readiness::Unavailable,
        });
        super::tests::draw(&app, 60, 18);
        let scope = app.held.inspection.as_ref().unwrap();
        let details = scope.details.get().unwrap();
        assert!(details.contains(&format!("{raw:?}")));
        assert!(
            !details
                .chars()
                .any(|ch| matches!(ch, '\r' | '\u{1b}' | '\u{202e}'))
        );
        assert_eq!(scope.content.as_deref(), Some(raw));
        assert!(app.memory.selected_ids.is_empty());
        assert!(app.sources.basket.is_empty());
    }
}
