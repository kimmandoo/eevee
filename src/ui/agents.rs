use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, centered, panel, row, selection_details, text};
use crate::{
    app::{App, agents::Page},
    config::Config,
};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 104, area.height.saturating_sub(2));
    frame.render_widget(Clear, area);
    let auth_room = app.width >= 80 && app.height >= 24;
    let state = &mut app.agents;
    if state.auth_preview.is_some() && !auth_room {
        state.reviewed = false;
        state.affirmative = false;
        state.review_choice = 0;
    }
    let block = panel(
        if state.auth_preview.is_some() {
            " Review connection "
        } else if state.preview {
            " Review version check "
        } else {
            " Connections "
        },
        config,
        paint,
        true,
    );
    let content = block.inner(area);
    frame.render_widget(block, area);
    let Some(profile) = state.profiles.get(state.selected) else {
        text(
            frame,
            content,
            "Checking installed providers…\nNo provider program launched. Esc Back",
            paint.surface,
        );
        return;
    };
    if !state.preview && state.auth_preview.is_none() {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(20)])
            .split(row(content, 0, content.height.saturating_sub(2)));
        text(frame, row(columns[0], 0, 1), "PROVIDERS", paint.accent);
        for (index, provider) in state.profiles.iter().enumerate() {
            let selected = index == state.selected;
            text(
                frame,
                row(columns[0], 1 + index as u16 * 2, 2),
                &format!(
                    "{} {}\n  {}",
                    if selected { ">" } else { " " },
                    provider.name,
                    state.connection_status(index)
                ),
                if selected && state.page == Page::Providers {
                    paint.accent
                } else {
                    paint.surface
                },
            );
        }
        let body = columns[1];
        text(
            frame,
            row(body, 0, 1),
            &format!(
                "{} · {}",
                profile.name,
                state.connection_status(state.selected)
            ),
            paint.accent,
        );
        if state.page == Page::Diagnostics {
            let mut value = format!(
                "{}\n\nInstalled: {}\nExecutable: {:?}\nArguments: {:?}\nVersion: {}\nAuthentication: {}\nBilling: {}\nAdapter: {}\nAction: {}\nHost: {}/{}\nContract revision: {}\n\n{}\n\n",
                state.setup_hint(),
                profile.installed,
                profile.executable,
                profile.argv,
                profile.version,
                profile.authentication,
                profile.billing,
                profile.adapter,
                profile.action,
                profile.os,
                profile.architecture,
                profile.revision,
                profile.configuration
            );
            for blocker in &profile.blockers {
                value.push_str(&format!("Blocker / remedy: {blocker}\n"));
            }
            for capability in &profile.capabilities {
                value.push_str(&format!(
                    "\n{}: {}\nEvidence: {}\nRestriction: {}\n",
                    capability.action, capability.boundary, capability.evidence, capability.blocker
                ));
            }
            state.scroll_max = selection_details(
                frame,
                row(body, 1, body.height.saturating_sub(1)),
                Paragraph::new(crate::privacy::redact(&value))
                    .style(paint.surface)
                    .wrap(Wrap { trim: false }),
                state.scroll,
            );
            state.scroll = state.scroll.min(state.scroll_max);
        } else {
            let methods = state.page == Page::Methods;
            text(
                frame,
                row(body, 2, 1),
                if methods {
                    "CHOOSE SIGN-IN METHOD"
                } else {
                    "ACTIONS"
                },
                paint.accent,
            );
            let count = if methods {
                for (index, method) in crate::auth::methods(state.selected).iter().enumerate() {
                    text(
                        frame,
                        row(body, 3 + index as u16 * 2, 2),
                        &format!(
                            "{} {}",
                            if index == state.method { ">" } else { " " },
                            method.label()
                        ),
                        if index == state.method {
                            paint.accent
                        } else {
                            paint.surface
                        },
                    );
                }
                crate::auth::methods(state.selected).len() as u16 * 2
            } else {
                for (index, action) in state.actions().iter().enumerate() {
                    text(
                        frame,
                        row(body, 3 + index as u16, 1),
                        &format!(
                            "{} {}",
                            if index == state.action { ">" } else { " " },
                            action.label()
                        ),
                        if index == state.action && state.page == Page::Actions {
                            paint.accent
                        } else {
                            paint.surface
                        },
                    );
                }
                state.actions().len() as u16
            };
            let hint = if state.selected == 1 || profile.executable.is_none() {
                state.setup_hint()
            } else if state.auth_busy() {
                "Preparing review… Esc cancels. No provider program launched.".into()
            } else {
                state.notice.clone()
            };
            let start = 4 + count;
            frame.render_widget(
                Paragraph::new(crate::privacy::redact(&hint))
                    .style(paint.surface)
                    .wrap(Wrap { trim: false }),
                row(body, start, body.height.saturating_sub(start)),
            );
        }
        text(
            frame,
            row(content, content.height.saturating_sub(2), 2),
            if state.page == Page::Diagnostics {
                "Up/Down Scroll · Enter/Esc Back · Tab Providers\nInstalled ≠ connected. Model requests need separate approval."
            } else {
                "Up/Down Select · Tab Providers/Actions · Enter Open · Esc Back\nConnect always opens a review first. No automatic model requests."
            },
            paint.accent,
        );
        return;
    }
    let value = if let Some(plan) = &state.auth_preview {
        if state.auth_details {
            plan.lines.join("\n\n")
        } else {
            plan.summary.join("\n")
        }
    } else {
        profile.trust_preview.join("\n\n")
    };
    let details = row(content, 0, content.height.saturating_sub(3));
    state.scroll_max = selection_details(
        frame,
        details,
        Paragraph::new(crate::privacy::redact(&value))
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll = state.scroll.min(state.scroll_max);
    if !state.auth_details
        && (state.auth_preview.is_none() || auth_room)
        && details.width >= 40
        && details.height >= 3
        && state.scroll >= state.scroll_max
    {
        state.reviewed = true;
    }
    let footer = if let Some(plan) = &state.auth_preview {
        if !auth_room {
            "Resize to 80 × 24 to review. Esc Back; nothing launches.".into()
        } else if state.auth_details {
            "[Back to summary]\nUp/Down Scroll · Enter Summary · Esc Cancel".into()
        } else {
            let approve = if plan.method.is_status() {
                "Check status"
            } else {
                "Connect"
            };
            let choice = if state.affirmative {
                1
            } else if state.review_choice == 2 {
                2
            } else {
                0
            };
            let labels = ["Back", approve, "Details"];
            let buttons = labels
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    if index == choice {
                        format!("[{label}]")
                    } else {
                        (*label).to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("   ");
            format!(
                "{buttons}\n{}",
                if state.reviewed {
                    "Tab Choose · Enter Confirm · Esc Back"
                } else {
                    "Up/Down Read remaining effects · Tab Details/Back · Esc Back"
                }
            )
        }
    } else if !state.reviewed {
        "[Back]   Check version (read remaining effects first)\nUp/Down Scroll · Esc Back".into()
    } else if state.affirmative {
        "Back   [Check version]\nTab Choose · Enter Confirm · Esc Back".into()
    } else {
        "[Back]   Check version\nTab Choose · Enter Confirm · Esc Back".into()
    };
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        &footer,
        paint.accent,
    );
}
