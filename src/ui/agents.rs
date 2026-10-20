use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, panel, row, selection_details, sheet, text};
use crate::{
    app::{
        App,
        agents::{ConnectionField, Page},
    },
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
    let area = sheet(area, 104, height);
    frame.render_widget(Clear, area);
    let auth_room = app.width >= 60 && app.height >= 18;
    let state = &mut app.agents;
    state.active = app.planning.agent.index();
    if state.auth_preview.is_some() && !auth_room {
        state.reviewed = false;
        state.affirmative = false;
        state.review_choice = 0;
    }
    let block = panel(
        if state.forget_preview.is_some() {
            " Forget app connection "
        } else if state.auth_preview.is_some() {
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
    if state.connection_editing() {
        render_connection(frame, content, state, paint);
        return;
    }
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
        let body = row(content, 0, content.height.saturating_sub(4));
        let capacity = usize::from(body.height.saturating_sub(1)).max(1);
        match state.page {
            Page::Providers => {
                text(
                    frame,
                    row(body, 0, 1),
                    &format!("Search providers: {}_", state.search),
                    paint.accent,
                );
                let indices = state.provider_indices();
                let selected = indices
                    .clone()
                    .position(|index| index == state.selected)
                    .unwrap_or(0);
                let first = selected.saturating_sub(capacity.saturating_sub(1));
                if indices.clone().next().is_none() {
                    text(
                        frame,
                        row(body, 1, 1),
                        "No matching route. Backspace edits; Esc clears.",
                        paint.muted,
                    );
                }
                for (offset, index) in indices.skip(first).take(capacity).enumerate() {
                    let agent = crate::providers::Agent::from_index(index);
                    let name =
                        agent.map_or(state.profiles[index].name.as_str(), |agent| agent.label());
                    text(
                        frame,
                        row(body, 1 + offset as u16, 1),
                        &format!(
                            "{}{} {} | {}",
                            if index == state.selected { ">" } else { " " },
                            if index == state.active { "*" } else { " " },
                            name,
                            state.connection_status(index)
                        ),
                        if index == state.selected {
                            paint.accent
                        } else {
                            paint.surface
                        },
                    );
                }
            }
            Page::Methods => {
                text(
                    frame,
                    row(body, 0, 1),
                    &format!("{} | sign-in method", profile.name),
                    paint.accent,
                );
                let methods = crate::providers::auth::methods(state.selected);
                for (index, method) in methods.iter().enumerate() {
                    text(
                        frame,
                        row(body, 1 + index as u16, 1),
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
                let start = 2 + methods.len() as u16;
                if let Some(guidance) = methods
                    .get(state.method)
                    .and_then(|method| method.guidance())
                {
                    state.scroll_max = selection_details(
                        frame,
                        row(body, start, body.height.saturating_sub(start)),
                        Paragraph::new(guidance)
                            .style(paint.muted)
                            .wrap(Wrap { trim: false }),
                        state.scroll,
                    );
                }
            }
            Page::Actions => {
                text(
                    frame,
                    row(body, 0, 1),
                    &format!(
                        "{} | {}",
                        profile.name,
                        state.connection_status(state.selected)
                    ),
                    paint.accent,
                );
                let actions = state.actions();
                let first = state.action.saturating_sub(capacity.saturating_sub(1));
                for (offset, (index, action)) in actions
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(capacity)
                    .enumerate()
                {
                    text(
                        frame,
                        row(body, 1 + offset as u16, 1),
                        &format!(
                            "{} {}",
                            if index == state.action { ">" } else { " " },
                            state.action_label(*action)
                        ),
                        if index == state.action {
                            paint.accent
                        } else {
                            paint.surface
                        },
                    );
                }
            }
            Page::Diagnostics => {
                let mut value = format!(
                    "{}\n{}\n\n{}\nInstalled: {}\nExecutable: {:?}\nArguments: {:?}\nVersion: {}\nAuthentication: {}\nBilling: {}\nAdapter: {}\nAdapter revision: {}\nProfile action: {}\n",
                    profile.name,
                    state.setup_hint(),
                    profile.configuration,
                    profile.installed,
                    profile.executable,
                    profile.argv,
                    profile.version,
                    profile.authentication,
                    profile.billing,
                    profile.adapter,
                    profile.revision,
                    profile.action
                );
                for blocker in &profile.blockers {
                    value.push_str(&format!("\nBlocker / remedy: {blocker}"));
                }
                for capability in &profile.capabilities {
                    value.push_str(&format!(
                        "\n{}: {}\nEvidence: {}\nRestriction: {}\n",
                        capability.action,
                        capability.boundary,
                        capability.evidence,
                        capability.blocker
                    ));
                }
                state.scroll_max = selection_details(
                    frame,
                    body,
                    Paragraph::new(crate::workspace::privacy::redact(&value))
                        .style(paint.surface)
                        .wrap(Wrap { trim: false }),
                    state.scroll,
                );
                state.scroll = state.scroll.min(state.scroll_max);
            }
        }
        text(
            frame,
            row(content, content.height.saturating_sub(4), 2),
            &crate::workspace::privacy::redact(&state.notice),
            paint.muted,
        );
        let footer = match state.page {
            Page::Providers => format!(
                "Type Search | Up/Down Select | Enter {}\nRight Options | * task route | Esc Back",
                state.enter_hint()
            ),
            Page::Methods => {
                "Up/Down Select | Enter Review method\nTab Providers | / choices | Esc Back".into()
            }
            Page::Actions => {
                "Up/Down Select | Enter Open action\nTab Providers | / choices | Esc Back".into()
            }
            Page::Diagnostics => "Up/Down Scroll | Tab Providers\n/ choices | Esc Back".into(),
        };
        text(
            frame,
            row(content, content.height.saturating_sub(2), 2),
            &footer,
            paint.accent,
        );
        return;
    }
    let details = row(content, 0, content.height.saturating_sub(1));
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
    let footer = if !auth_room || !state.reviewed && !state.auth_details {
        "Scope not fully visible | d Details | Esc Back".into()
    } else if state.auth_details {
        "d Summary | PgUp/Dn Scroll | Esc Cancel".into()
    } else if let Some(plan) = &state.auth_preview {
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
            .join(" ");
        format!("{buttons} | y/n d Tab Enter")
    } else if state.affirmative {
        "Back [Check version] | y/n d Tab Enter".into()
    } else {
        "[Back] Check version | y/n d Tab Enter".into()
    };
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        &footer,
        paint.accent,
    );
}

fn render_connection(
    frame: &mut Frame,
    content: Rect,
    state: &mut crate::app::agents::State,
    paint: &Paint,
) {
    if state.forget_preview.is_some() {
        render_forget(frame, content, state, paint);
        return;
    }
    let Some(agent) = crate::providers::Agent::from_index(state.selected) else {
        return;
    };
    let fields = state.connection_fields();
    let notice = &state.notice;
    let editor = state
        .connection
        .as_mut()
        .expect("private connection editor");
    let title = if agent == crate::providers::Agent::ChatGPT {
        "Codex native | official ChatGPT only | no tools".into()
    } else {
        format!("{} | native; no tools", agent.label())
    };
    text(frame, row(content, 0, 1), &title, paint.accent);
    let room = content.width >= 54 && content.height >= 12;
    editor.visible = room;
    let has_effort = editor.form.effort.is_some();
    let capacity = usize::from(
        content
            .height
            .saturating_sub(if has_effort { 7 } else { 6 }),
    )
    .max(1);
    let first = editor.field.saturating_sub(capacity.saturating_sub(1));
    for (offset, (index, field)) in fields
        .iter()
        .enumerate()
        .skip(first)
        .take(capacity)
        .enumerate()
    {
        let (label, value, private) = match field {
            ConnectionField::Credential => (
                if agent == crate::providers::Agent::ChatGPT {
                    "Access token"
                } else {
                    "API key"
                },
                if editor.form.credential.is_empty() {
                    "(blank keeps selected source)".into()
                } else {
                    "********".into()
                },
                true,
            ),
            ConnectionField::Endpoint => (
                "Endpoint",
                crate::workspace::privacy::redact(&editor.form.base_url),
                false,
            ),
            ConnectionField::Model => (
                "Default model",
                if editor.form.model.is_empty() {
                    "(explicit choice later)".into()
                } else {
                    crate::workspace::privacy::redact(&editor.form.model)
                },
                false,
            ),
            ConnectionField::Account => (
                "Account ID",
                if editor.form.account_id.is_empty() {
                    "(token/source account)".into()
                } else {
                    "********".into()
                },
                true,
            ),
            ConnectionField::Protocol => (
                "Protocol",
                if editor.form.responses {
                    "responses"
                } else {
                    "chat_completions"
                }
                .into(),
                false,
            ),
            ConnectionField::Keyless => (
                "Authentication",
                if editor.form.keyless {
                    "KEYLESS: no authentication header"
                } else {
                    "API key (separate from endpoint/model)"
                }
                .into(),
                false,
            ),
            ConnectionField::Storage => (
                "Storage",
                if editor.form.persist {
                    "SAVE private local plaintext"
                } else {
                    "SESSION (older saved profile unchanged)"
                }
                .into(),
                false,
            ),
            ConnectionField::Save => (
                "Save & choose model",
                "Select route; metadata only".into(),
                false,
            ),
            ConnectionField::Browser => (
                "Browser login",
                "Own token stays; official Codex CLI".into(),
                false,
            ),
            ConnectionField::Device => (
                "Device login",
                "Own token stays; official Codex CLI".into(),
                false,
            ),
            ConnectionField::LoadCli => (
                "Load CLI token",
                "Read-only auth.json into draft; then Save".into(),
                false,
            ),
            ConnectionField::Forget => (
                "Forget app connection",
                "Review own override deletion only".into(),
                false,
            ),
        };
        let prefix = format!(
            "{} {label}: ",
            if editor.field == index { ">" } else { " " }
        );
        let text_field = matches!(
            field,
            ConnectionField::Credential
                | ConnectionField::Endpoint
                | ConnectionField::Model
                | ConnectionField::Account
        );
        let capacity = usize::from(content.width)
            .saturating_sub(prefix.len())
            .max(1);
        let displayed = if private || value.chars().count() <= capacity {
            value.clone()
        } else {
            let prefix_chars = value
                .char_indices()
                .take_while(|(index, _)| *index < editor.cursor)
                .count();
            let start = if editor.field == index {
                prefix_chars.saturating_sub(capacity.saturating_sub(1))
            } else {
                0
            };
            value.chars().skip(start).take(capacity).collect()
        };
        text(
            frame,
            row(content, 1 + offset as u16, 1),
            &format!("{prefix}{displayed}"),
            if editor.field == index {
                paint.accent
            } else {
                paint.surface
            },
        );
        if text_field && editor.field == index {
            let caret_column = if private {
                if editor.form.credential.is_empty() && *field == ConnectionField::Credential
                    || editor.form.account_id.is_empty() && *field == ConnectionField::Account
                {
                    0
                } else {
                    8
                }
            } else {
                editor.cursor.min(capacity.saturating_sub(1))
            };
            if 1 + (offset as u16) < content.height && content.width > 0 {
                frame.set_cursor_position((
                    content.x
                        + (prefix.len() + caret_column)
                            .min(content.width.saturating_sub(1) as usize)
                            as u16,
                    content.y + 1 + offset as u16,
                ));
            }
        }
    }
    if has_effort {
        let effort = if editor.form.effort_resets() {
            "Model changed: Save resets default effort; choose anew.".into()
        } else {
            format!(
                "Default effort: {} (same model only)",
                crate::workspace::privacy::redact(
                    editor.form.effort.as_deref().unwrap_or_default()
                )
            )
        };
        text(
            frame,
            row(content, content.height.saturating_sub(6), 1),
            &effort,
            paint.accent,
        );
    }
    let disclosure = "Save+model: select route + metadata; NEVER send task.\nSaved plaintext; NOT encrypted/keychain. Ctrl+S local.";
    text(
        frame,
        row(content, content.height.saturating_sub(5), 2),
        disclosure,
        paint.muted,
    );
    state.scroll_max = selection_details(
        frame,
        row(content, content.height.saturating_sub(3), 2),
        Paragraph::new(crate::workspace::privacy::redact(notice))
            .style(paint.muted)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll = state.scroll.min(state.scroll_max);
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        if room {
            "Tab Fields | Enter | Ctrl+S Local | PgUp/Dn | Esc"
        } else {
            "Resize to 60x18 | Esc Cancel; nothing saved"
        },
        paint.accent,
    );
}

fn render_forget(
    frame: &mut Frame,
    content: Rect,
    state: &mut crate::app::agents::State,
    paint: &Paint,
) {
    let summary = state
        .forget_preview
        .as_ref()
        .expect("private deletion review")
        .summary();
    state.reviewed = content.width >= 54
        && content.height >= 10
        && super::planning::render_summary(
            frame,
            row(content, 0, content.height.saturating_sub(3)),
            &summary,
            paint,
        );
    if !state.reviewed {
        state.affirmative = false;
    }
    text(
        frame,
        row(content, content.height.saturating_sub(3), 2),
        &crate::workspace::privacy::redact(&state.notice),
        paint.muted,
    );
    let footer = if !state.reviewed {
        "Resize to 60x18 for full scope | Esc Cancel"
    } else if state.affirmative {
        "Back [Forget local] | Tab Enter | y/n | Esc"
    } else {
        "[Back] Forget local | Tab Enter | y/n | Esc"
    };
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        footer,
        paint.accent,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, style::Style};

    #[test]
    fn private_setup_masks_paste_and_shows_storage_scope_at_supported_sizes() {
        let style = Style::default();
        let paint = Paint {
            base: style,
            surface: style,
            muted: style,
            border: style,
            accent: style,
            selected: style,
        };
        for (width, height) in [(60, 18), (120, 36)] {
            for agent in [
                crate::providers::Agent::ChatGPT,
                crate::providers::Agent::Compatible,
            ] {
                let mut state = crate::app::agents::State::new();
                state.selected = agent.index();
                state.connection = Some(crate::app::agents::ConnectionEditor {
                    form: crate::providers::direct::config::Setup {
                        credential: String::new(),
                        base_url: "https://api.example/v1".into(),
                        model: "chosen-model".into(),
                        effort: None,
                        effort_model: None,
                        account_id: "private-account-sentinel".into(),
                        responses: true,
                        keyless: false,
                        persist: false,
                        source_notice: None,
                    },
                    field: 0,
                    cursor: 0,
                    visible: false,
                });
                assert!(state.paste_connection("private-credential-sentinel"));
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal
                    .draw(|frame| {
                        render_connection(
                            frame,
                            Rect::new(2, 3, width - 4, height - 6),
                            &mut state,
                            &paint,
                        )
                    })
                    .unwrap();
                let display: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(!display.contains("private-credential-sentinel"));
                assert!(!display.contains("private-account-sentinel"));
                assert!(display.contains("********"));
                assert!(display.contains("SESSION"));
                assert!(display.contains("NOT encrypted/keychain"));
                assert!(state.connection.as_ref().unwrap().visible);
                state.connection.as_mut().unwrap().field = state
                    .connection_fields()
                    .iter()
                    .position(|field| *field == ConnectionField::Save)
                    .unwrap();
                terminal
                    .draw(|frame| {
                        render_connection(
                            frame,
                            Rect::new(2, 3, width - 4, height - 6),
                            &mut state,
                            &paint,
                        )
                    })
                    .unwrap();
                let display: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                assert!(display.contains("Save & choose model"));
                assert!(display.contains("NEVER send task"));
                assert!(!display.contains("private-credential-sentinel"));
                if agent == crate::providers::Agent::ChatGPT {
                    let editor = state.connection.as_mut().unwrap();
                    editor.form.effort = Some("high".into());
                    editor.form.effort_model = Some("chosen-model".into());
                    editor.form.model = "different-model".into();
                    terminal
                        .draw(|frame| {
                            render_connection(
                                frame,
                                Rect::new(2, 3, width - 4, height - 6),
                                &mut state,
                                &paint,
                            )
                        })
                        .unwrap();
                    let display: String = terminal
                        .backend()
                        .buffer()
                        .content
                        .iter()
                        .map(|cell| cell.symbol())
                        .collect();
                    assert!(display.contains("Save resets default effort"));
                    assert!(display.contains("Save & choose model"));
                    assert!(display.contains("NOT encrypted/keychain"));
                    assert_eq!(
                        state.connection.as_ref().unwrap().form.effort.as_deref(),
                        Some("high")
                    );
                }
            }
        }
    }
}
