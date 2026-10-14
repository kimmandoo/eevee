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
            "Checking installed providers...\nEsc Back",
            paint.surface,
        );
        return;
    };
    if !state.preview && state.auth_preview.is_none() {
        let narrow = content.width < 72;
        let body_area = row(content, 0, content.height.saturating_sub(2));
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(25), Constraint::Min(20)])
            .split(body_area);
        let show_providers = !narrow || state.page == Page::Providers;
        if show_providers {
            let providers = if narrow { body_area } else { columns[0] };
            text(frame, row(providers, 0, 1), "PROVIDERS", paint.accent);
            for (index, provider) in state.profiles.iter().enumerate() {
                text(
                    frame,
                    row(providers, 1 + index as u16 * 2, 2),
                    &format!(
                        "{} {}\n  {}",
                        if index == state.selected { ">" } else { " " },
                        provider.name,
                        state.connection_status(index)
                    ),
                    if index == state.selected && state.page == Page::Providers {
                        paint.accent
                    } else {
                        paint.surface
                    },
                );
            }
        }
        let body = if narrow { body_area } else { columns[1] };
        let provider_list = narrow && state.page == Page::Providers;
        if !provider_list {
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
        }
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
            let start = if provider_list {
                1 + state.profiles.len() as u16 * 2
            } else {
                let methods = state.page == Page::Methods;
                text(
                    frame,
                    row(body, 1, 1),
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
                            row(body, 2 + index as u16, 1),
                            &format!(
                                "{} {}{}",
                                if index == state.method { ">" } else { " " },
                                method.label(),
                                if index == crate::auth::default_method(state.selected) {
                                    " (default)"
                                } else {
                                    ""
                                }
                            ),
                            if index == state.method {
                                paint.accent
                            } else {
                                paint.surface
                            },
                        );
                    }
                    crate::auth::methods(state.selected).len() as u16
                } else {
                    for (index, action) in state.actions().iter().enumerate() {
                        text(
                            frame,
                            row(body, 2 + index as u16, 1),
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
                3 + count
            };
            let methods = state.page == Page::Methods;
            // Claude's executable row is the Claude CLI; its connection uses ant instead.
            let hint = if state.selected != 1 && profile.executable.is_none() {
                state.setup_hint()
            } else if state.auth_busy() {
                "Preparing review... Esc cancels.".into()
            } else if methods {
                let guidance = crate::auth::methods(state.selected)
                    .get(state.method)
                    .and_then(|method| method.guidance())
                    .unwrap_or("Enter reviews this method; nothing launches.");
                format!("{}\n\n{guidance}", state.notice)
            } else if state.selected == 1 {
                format!("{}\n\n{}", state.notice, state.setup_hint())
            } else {
                state.notice.clone()
            };
            state.scroll_max = selection_details(
                frame,
                row(body, start, body.height.saturating_sub(start)),
                Paragraph::new(crate::privacy::redact(&hint))
                    .style(paint.surface)
                    .wrap(Wrap { trim: false }),
                state.scroll,
            );
            state.scroll = state.scroll.min(state.scroll_max);
        }
        text(
            frame,
            row(content, content.height.saturating_sub(2), 2),
            if state.page == Page::Diagnostics {
                "Up/Down Scroll · Enter Back · / choices · Esc Back"
            } else {
                "Arrows Select · Tab Providers/Actions · Enter Open\n/ choices: connect, status, version, diagnostics · Esc Back"
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
