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
    i18n::{Message, Template},
};

fn setup_hint(selected: usize, paint: &Paint) -> &'static str {
    use crate::providers::Agent;
    paint.hint(match Agent::from_index(selected) {
        Some(Agent::Codex) => Message::ProvidersSetupCodex,
        Some(Agent::Claude) => Message::ProvidersSetupClaude,
        Some(Agent::OpenCode) => Message::ProvidersSetupOpencode,
        Some(Agent::Gemini) => Message::ProvidersSetupGemini,
        Some(Agent::Devin) => Message::ProvidersSetupDevin,
        Some(Agent::ChatGPT) => Message::ProvidersSetupChatgpt,
        Some(Agent::OpenAI) => Message::ProvidersSetupOpenai,
        Some(Agent::Anthropic) => Message::ProvidersSetupAnthropic,
        Some(Agent::Google) => Message::ProvidersSetupGoogle,
        Some(Agent::Compatible) => Message::ProvidersSetupCompatible,
        None => Message::ProvidersSetupNone,
    })
}

fn method_label(method: crate::providers::auth::Method, paint: &Paint) -> &'static str {
    use crate::providers::auth::Method;
    paint.hint(match method {
        Method::CodexBrowser => Message::ProvidersAuthLabelCodexBrowser,
        Method::CodexDevice => Message::ProvidersAuthLabelCodexDevice,
        Method::OpenCode => Message::ProvidersAuthLabelOpenCode,
        Method::GeminiNative => Message::ProvidersAuthLabelGeminiNative,
        Method::ClaudeConsole => Message::ProvidersAuthLabelClaudeConsole,
        Method::ClaudeConsoleCode => Message::ProvidersAuthLabelClaudeConsoleCode,
        Method::DevinWeb => Message::ProvidersAuthLabelDevinWeb,
        Method::DevinBrowser => Message::ProvidersAuthLabelDevinBrowser,
        Method::DevinManualToken => Message::ProvidersAuthLabelDevinManualToken,
        Method::DevinStatus => Message::ProvidersAuthLabelDevinStatus,
        Method::ClaudeConsoleStatus => Message::ProvidersAuthLabelClaudeConsoleStatus,
        Method::CodexStatus => Message::ProvidersAuthLabelCodexStatus,
        Method::OpenCodeStatus => Message::ProvidersAuthLabelOpenCodeStatus,
    })
}

fn method_guidance(method: crate::providers::auth::Method, paint: &Paint) -> Option<&'static str> {
    use crate::providers::auth::Method;
    Some(paint.hint(match method {
        Method::CodexBrowser => Message::ProvidersAuthGuidanceCodexBrowser,
        Method::CodexDevice => Message::ProvidersAuthGuidanceCodexDevice,
        Method::ClaudeConsole | Method::ClaudeConsoleCode => {
            Message::ProvidersAuthGuidanceClaudeConsoleCode
        }
        Method::DevinWeb => Message::ProvidersAuthGuidanceDevinWeb,
        Method::DevinBrowser | Method::DevinManualToken => {
            Message::ProvidersAuthGuidanceDevinManualToken
        }
        _ => return None,
    }))
}

fn action_hint<'a>(paint: &Paint, action: &'a str) -> &'a str {
    let message = match action {
        "Connect / credential setup" => Message::ProvidersActionConnectCredentialSetup,
        "Check connection status" => Message::ProvidersActionCheckConnectionStatus,
        "Check installed version" => Message::ProvidersActionCheckInstalledVersion,
        "Advanced diagnostics" => Message::ProvidersActionAdvancedDiagnostics,
        "Refresh installed providers" => Message::ProvidersActionRefreshInstalledProviders,
        "Use for task" => Message::ProvidersActionUseForTask,
        "Choose model" => Message::ProvidersActionChooseModel,
        "Choose reasoning effort" => Message::ProvidersActionChooseReasoningEffort,
        "Change Codex connection method" => Message::ProvidersActionChangeCodexConnectionMethod,
        "Set up native responses (scoped coding)" => {
            Message::ProvidersActionSetUpNativeResponsesScopedCoding
        }
        "Set up CLI agent (tools / edits)" => Message::ProvidersActionSetUpCliAgentToolsEdits,
        "Setup" => Message::ProvidersActionSetup,
        _ => return action,
    };
    paint.hint(message)
}

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
            paint.hint(Message::ProvidersAgentsForgetAppConnection2)
        } else if state.auth_preview.is_some() {
            paint.hint(Message::ProvidersAgentsReviewConnection)
        } else if state.preview {
            paint.hint(Message::ProvidersAgentsReviewVersionCheck)
        } else {
            paint.hint(Message::ProvidersAgentsConnections)
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
            paint.hint(Message::ProvidersAgentsCheckingInstalledProvidersEscBack),
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
                    &paint.format(
                        Template::ProvidersAgentsSearchProviders,
                        &[("query", &(state.search))],
                    ),
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
                        paint.hint(Message::ProvidersAgentsNoMatchingRouteBackspaceEditsEscClears),
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
                    &paint.format(
                        Template::ProvidersAgentsSignInMethod,
                        &[("provider", &(profile.name))],
                    ),
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
                            method_label(*method, paint)
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
                    .and_then(|method| method_guidance(*method, paint))
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
                            action_hint(paint, state.action_label(*action))
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
                    setup_hint(state.selected, paint),
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
            Page::Providers => paint.format(
                Template::ProvidersAgentsTypeSearchUpDownSelectEnterRight,
                &[("action", (action_hint(paint, state.enter_hint())))],
            ),
            Page::Methods => paint
                .hint(Message::ProvidersAgentsUpDownSelectEnterReviewMethodTab)
                .into(),
            Page::Actions => paint
                .hint(Message::ProvidersAgentsUpDownSelectEnterOpenActionTab)
                .into(),
            Page::Diagnostics => paint
                .hint(Message::ProvidersAgentsUpDownScrollTabProvidersChoicesEsc)
                .into(),
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
                paint
                    .hint(Message::ProvidersAgentsRunsExecutableWithVersionArgumentNoTask)
                    .into(),
                paint
                    .hint(Message::ProvidersAgentsTrustExecutableAndInheritedEnvironmentConfig)
                    .into(),
                paint
                    .hint(Message::ProvidersAgentsNotSandboxedExecutableMayHaveUnknownEffects)
                    .into(),
                if profile.probe_allowed {
                    paint
                        .hint(Message::ProvidersAgentsOneBoundedCheckDoesNotEstablishAuthentication)
                        .into()
                } else {
                    paint
                        .hint(Message::ProvidersAgentsBlockedExecutablePrerequisitesNotSatisfied)
                        .into()
                },
                paint
                    .hint(Message::ProvidersAgentsDDetailsExactExecutableConfigurationAndRisks)
                    .into(),
            ];
            &version_summary
        };
        state.reviewed =
            auth_room && super::planning::render_summary(frame, details, summary, paint);
        state.scroll = 0;
        state.scroll_max = 0;
    }
    let footer = if !auth_room || !state.reviewed && !state.auth_details {
        paint
            .hint(Message::ProvidersAgentsScopeNotFullyVisibleDDetailsEsc)
            .into()
    } else if state.auth_details {
        paint
            .hint(Message::ProvidersAgentsDSummaryPgupDnScrollEscCancel)
            .into()
    } else if let Some(plan) = &state.auth_preview {
        let approve = if plan.method.is_status() {
            paint.hint(Message::CommonCheckStatus)
        } else {
            paint.hint(Message::CommonConnect)
        };
        let choice = if state.affirmative {
            1
        } else if state.review_choice == 2 {
            2
        } else {
            0
        };
        let labels = [
            paint.hint(Message::CommonBack),
            approve,
            paint.hint(Message::CommonDetails),
        ];
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
        paint.format(
            Template::ProvidersAgentsButtonsYNDTabEnter,
            &[("buttons", &(buttons))],
        )
    } else if state.affirmative {
        paint
            .hint(Message::ProvidersAgentsBackCheckVersionYNDTab)
            .into()
    } else {
        paint
            .hint(Message::ProvidersAgentsBackCheckVersionYNDTab2)
            .into()
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
        paint
            .hint(Message::ProvidersAgentsCodexNativeOfficialChatgptScopedCoding)
            .into()
    } else {
        paint.format(
            Template::ProvidersAgentsNativeScopedCoding,
            &[("provider", (agent.label()))],
        )
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
                    paint.hint(Message::ProvidersAgentsAccessToken)
                } else {
                    paint.hint(Message::ProvidersAgentsApiKey)
                },
                if editor.form.credential.is_empty() {
                    paint
                        .hint(Message::ProvidersAgentsBlankKeepsSelectedSource)
                        .into()
                } else {
                    "********".into()
                },
                true,
            ),
            ConnectionField::Endpoint => (
                paint.hint(Message::ProvidersAgentsEndpoint),
                crate::workspace::privacy::redact(&editor.form.base_url),
                false,
            ),
            ConnectionField::Model => (
                paint.hint(Message::ProvidersAgentsDefaultModel),
                if editor.form.model.is_empty() {
                    paint
                        .hint(Message::ProvidersAgentsExplicitChoiceLater)
                        .into()
                } else {
                    crate::workspace::privacy::redact(&editor.form.model)
                },
                false,
            ),
            ConnectionField::Account => (
                paint.hint(Message::ProvidersAgentsAccountId),
                if editor.form.account_id.is_empty() {
                    paint
                        .hint(Message::ProvidersAgentsTokenSourceAccount)
                        .into()
                } else {
                    "********".into()
                },
                true,
            ),
            ConnectionField::Protocol => (
                paint.hint(Message::ProvidersAgentsProtocol),
                if editor.form.responses {
                    "responses"
                } else {
                    "chat_completions"
                }
                .into(),
                false,
            ),
            ConnectionField::Keyless => (
                paint.hint(Message::ProvidersAgentsAuthentication),
                if editor.form.keyless {
                    paint.hint(Message::ProvidersAgentsKeylessNoAuthenticationHeader)
                } else {
                    paint.hint(Message::ProvidersAgentsApiKeySeparateFromEndpointModel)
                }
                .into(),
                false,
            ),
            ConnectionField::Storage => (
                paint.hint(Message::ProvidersAgentsStorage),
                if editor.form.persist {
                    paint.hint(Message::ProvidersAgentsSavePrivateLocalPlaintext)
                } else {
                    paint.hint(Message::ProvidersAgentsSessionOlderSavedProfileUnchanged)
                }
                .into(),
                false,
            ),
            ConnectionField::Save => (
                paint.hint(Message::ProvidersAgentsSaveChooseModel),
                paint
                    .hint(Message::ProvidersAgentsSelectRouteMetadataOnly)
                    .into(),
                false,
            ),
            ConnectionField::Browser => (
                paint.hint(Message::ProvidersAgentsBrowserLogin),
                paint
                    .hint(Message::ProvidersAgentsOwnTokenStaysOfficialCodexCli)
                    .into(),
                false,
            ),
            ConnectionField::Device => (
                paint.hint(Message::ProvidersAgentsDeviceLogin),
                paint
                    .hint(Message::ProvidersAgentsOwnTokenStaysOfficialCodexCli)
                    .into(),
                false,
            ),
            ConnectionField::LoadCli => (
                paint.hint(Message::ProvidersAgentsLoadCliToken),
                paint
                    .hint(Message::ProvidersAgentsReadOnlyAuthJsonIntoDraftThen)
                    .into(),
                false,
            ),
            ConnectionField::Forget => (
                paint.hint(Message::ProvidersAgentsForgetAppConnection),
                paint
                    .hint(Message::ProvidersAgentsReviewOwnOverrideDeletionOnly)
                    .into(),
                false,
            ),
        };
        let prefix = format!(
            "{} {label}: ",
            if editor.field == index { ">" } else { " " }
        );
        let prefix_width = ratatui::text::Span::raw(prefix.as_str()).width();
        let text_field = matches!(
            field,
            ConnectionField::Credential
                | ConnectionField::Endpoint
                | ConnectionField::Model
                | ConnectionField::Account
        );
        let capacity = usize::from(content.width)
            .saturating_sub(prefix_width)
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
                        + (prefix_width + caret_column)
                            .min(content.width.saturating_sub(1) as usize)
                            as u16,
                    content.y + 1 + offset as u16,
                ));
            }
        }
    }
    if has_effort {
        let effort = if editor.form.effort_resets() {
            paint
                .hint(Message::ProvidersAgentsModelChangedSaveResetsDefaultEffortChoose)
                .into()
        } else {
            paint.format(
                Template::ProvidersAgentsDefaultEffortSameModelOnly,
                &[(
                    "effort",
                    &(crate::workspace::privacy::redact(
                        editor.form.effort.as_deref().unwrap_or_default(),
                    )),
                )],
            )
        };
        text(
            frame,
            row(content, content.height.saturating_sub(6), 1),
            &effort,
            paint.accent,
        );
    }
    let disclosure = paint.hint(Message::ProvidersAgentsSaveModelSelectRouteMetadataNeverSend);
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
            paint.hint(Message::ProvidersAgentsTabFieldsEnterCtrlSLocalPgup)
        } else {
            paint.hint(Message::ProvidersAgentsResizeTo60x18EscCancelNothingSaved)
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
        paint.hint(Message::ProvidersAgentsResizeTo60x18ForFullScopeEsc)
    } else if state.affirmative {
        paint.hint(Message::ProvidersAgentsBackForgetLocalTabEnterYN)
    } else {
        paint.hint(Message::ProvidersAgentsBackForgetLocalTabEnterYN2)
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
            language: crate::i18n::Language::ENGLISH,
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
