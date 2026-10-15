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
    let compact =
        (app.agents.preview || app.agents.auth_preview.is_some()) && !app.agents.auth_details;
    let height = if compact {
        18
    } else {
        area.height.saturating_sub(2)
    };
    let area = centered(area, 104, height);
    frame.render_widget(Clear, area);
    let auth_room = app.width >= 60 && app.height >= 18;
    let in_use = format!("in use · {}", app.planning.selection.summary());
    let state = &mut app.agents;
    state.active = app.planning.agent.index();
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
                        "{} {}{}\n  {}",
                        if index == state.selected { ">" } else { " " },
                        provider.name,
                        if index == state.active { " ●" } else { "" },
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
                &if state.selected == state.active {
                    format!(
                        "{} · {} · {in_use}",
                        profile.name,
                        state.connection_status(state.selected)
                    )
                } else {
                    format!(
                        "{} · {}",
                        profile.name,
                        state.connection_status(state.selected)
                    )
                },
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
                Paragraph::new(crate::workspace::privacy::redact(&value))
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
                    for (index, method) in crate::providers::auth::methods(state.selected)
                        .iter()
                        .enumerate()
                    {
                        text(
                            frame,
                            row(body, 2 + index as u16, 1),
                            &format!(
                                "{} {}{}",
                                if index == state.method { ">" } else { " " },
                                method.label(),
                                if index == crate::providers::auth::default_method(state.selected) {
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
                    crate::providers::auth::methods(state.selected).len() as u16
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
            let hint = if state.selected != crate::providers::Agent::Claude.index()
                && profile.executable.is_none()
            {
                super::brief(&state.setup_hint()).into_owned()
            } else if state.auth_busy() {
                "Preparing review... Esc cancels.".into()
            } else if methods {
                let guidance = crate::providers::auth::methods(state.selected)
                    .get(state.method)
                    .and_then(|method| method.guidance())
                    .unwrap_or("Enter reviews this method; nothing launches.");
                format!(
                    "{}\n\n{}",
                    super::brief(&state.notice),
                    super::brief(guidance)
                )
            } else if state.selected == crate::providers::Agent::Claude.index() {
                format!(
                    "{}\n\n{}",
                    super::brief(&state.notice),
                    super::brief(&state.setup_hint())
                )
            } else {
                super::brief(&state.notice).into_owned()
            };
            state.scroll_max = selection_details(
                frame,
                row(body, start, body.height.saturating_sub(start)),
                Paragraph::new(crate::workspace::privacy::redact(&hint))
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
                "Arrows Select · Tab Providers/Actions · Enter Open (suggested step preselected)\n● task provider · / choices: connect, status, version, diagnostics · Esc Back"
            },
            paint.accent,
        );
        return;
    }
    let details = row(content, 0, content.height.saturating_sub(3));
    if state.auth_details {
        let value = if let Some(plan) = &state.auth_preview {
            plan.lines.join("\n\n")
        } else {
            profile.trust_preview.join("\n\n")
        };
        state.scroll_max = selection_details(
            frame,
            details,
            Paragraph::new(crate::workspace::privacy::redact(&value))
                .style(paint.surface)
                .wrap(Wrap { trim: false }),
            state.scroll,
        );
        state.scroll = state.scroll.min(state.scroll_max);
        state.reviewed = false;
    } else {
        let version_summary;
        let summary = if let Some(plan) = &state.auth_preview {
            &plan.summary
        } else {
            version_summary = vec![
                format!("{} · version check only", profile.name),
                format!("Executable: {:?}", profile.executable),
                "Runs executable with version argument; no task sent.".into(),
                "Trust executable and inherited environment/config.".into(),
                "Not sandboxed; executable may have unknown effects.".into(),
                if profile.probe_allowed {
                    "One bounded check; does not establish authentication.".into()
                } else {
                    "BLOCKED: executable prerequisites not satisfied.".into()
                },
                "d Details: exact executable/configuration and risks.".into(),
            ];
            &version_summary
        };
        state.reviewed =
            auth_room && super::planning::render_summary(frame, details, summary, paint);
        state.scroll = 0;
        state.scroll_max = 0;
    }
    let footer = if let Some(plan) = &state.auth_preview {
        if !auth_room {
            "Resize to 60 × 18 to review. Esc Back; nothing launches.".into()
        } else if state.auth_details {
            "d Summary / PgUp/Dn Details / Esc Cancel\nReturn to summary to choose; nothing launches.".into()
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
                    "y Confirm / n Back / d Details · Tab / Enter"
                } else {
                    "Resize to show scope · d Details · Esc Back"
                }
            )
        }
    } else if state.auth_details {
        "d Summary / PgUp/Dn Details / Esc Back\nReturn to summary to choose; nothing launches."
            .into()
    } else if !state.reviewed {
        "[Back]   Check version unavailable until summary fits\nResize / Esc Back".into()
    } else if state.affirmative {
        "Back   [Check version] · y Check / n Back\nTab Choose · Enter Confirm · d Details · Esc Back".into()
    } else {
        "[Back]   Check version · y Check / n Back\nTab Choose · Enter Confirm · d Details · Esc Back".into()
    };
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        &footer,
        paint.accent,
    );
}
