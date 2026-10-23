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

fn readiness_label(state: &crate::app::agents::State, index: usize, paint: &Paint) -> &'static str {
    use crate::app::agents::Readiness;
    paint.hint(match state.readiness(index) {
        Readiness::Checking => Message::ProvidersStatusChecking,
        Readiness::Configured => Message::ProvidersStatusConfigured,
        Readiness::Setup => Message::ProvidersStatusSetup,
        Readiness::MissingCli => Message::ProvidersStatusMissing,
        Readiness::Blocked => Message::DiagnosticsStatusBlocked,
        Readiness::SignedIn => Message::ProvidersStatusSignedIn,
        Readiness::Session => Message::ProvidersStatusSession,
        Readiness::SignedOut => Message::ProvidersStatusSignedOut,
        Readiness::Unknown => Message::ProvidersStatusUnknown,
    })
}

fn diagnostic_reason(state: &crate::app::agents::State, paint: &Paint) -> &'static str {
    use crate::providers::diagnostics::Kind;
    if state.missing_executable(state.selected) {
        return paint.hint(Message::DiagnosticsMissing);
    }
    paint.hint(
        match state.failure(state.selected).map(|failure| failure.kind()) {
            Some(Kind::Authentication) => Message::DiagnosticsAuthentication,
            Some(Kind::Eligibility) => Message::DiagnosticsEligibility,
            Some(Kind::Endpoint) => Message::DiagnosticsEndpoint,
            Some(Kind::RateLimit) => Message::DiagnosticsRateLimit,
            Some(Kind::Request) => Message::DiagnosticsRequest,
            Some(Kind::Transport) => Message::DiagnosticsTransport,
            Some(Kind::Incompatible) => Message::DiagnosticsIncompatible,
            Some(Kind::VersionCheck) => Message::DiagnosticsVersionCheck,
            Some(Kind::LoginCancelled) => Message::DiagnosticsLoginCancelled,
            Some(Kind::LoginExpired) => Message::DiagnosticsLoginExpired,
            Some(Kind::LoginFailed) => Message::DiagnosticsLoginFailed,
            Some(Kind::LoginRefused) => Message::DiagnosticsLoginRefused,
            None => Message::DiagnosticsUnknown,
        },
    )
}

fn diagnostic_next(state: &crate::app::agents::State, paint: &Paint) -> &'static str {
    use crate::providers::{Agent, diagnostics::Kind};
    let agent = Agent::from_index(state.selected);
    let kind = state.failure(state.selected).map(|failure| failure.kind());
    paint.hint(
        if state.missing_executable(state.selected)
            || (matches!(
                kind,
                Some(Kind::Incompatible | Kind::VersionCheck | Kind::LoginRefused)
            ) && !agent.is_some_and(Agent::is_direct))
        {
            Message::DiagnosticsNextInstall
        } else if matches!(
            kind,
            Some(Kind::Eligibility | Kind::Endpoint | Kind::RateLimit | Kind::Request)
        ) {
            Message::DiagnosticsNextSettings
        } else if matches!(kind, Some(Kind::Transport)) {
            Message::DiagnosticsNextSetup
        } else {
            match agent {
                Some(Agent::ChatGPT) => Message::DiagnosticsNextAccount,
                Some(Agent::Claude) => Message::DiagnosticsNextClaude,
                Some(Agent::Gemini) => Message::DiagnosticsNextGemini,
                Some(Agent::Devin) => Message::DiagnosticsNextDevin,
                Some(agent) if agent.is_direct() => Message::DiagnosticsNextApi,
                _ => Message::DiagnosticsNextLogin,
            }
        },
    )
}

/// Keep the complete blocker and next step above the independently scrolling details.
fn diagnostic_summary(
    frame: &mut Frame,
    area: Rect,
    state: &crate::app::agents::State,
    paint: &Paint,
) -> u16 {
    let summary = Paragraph::new(format!(
        "{}\n{}",
        diagnostic_reason(state, paint),
        diagnostic_next(state, paint)
    ))
    .style(paint.accent)
    .wrap(Wrap { trim: false });
    let height = (summary.line_count(area.width.max(1)) as u16).min(area.height);
    frame.render_widget(summary, row(area, 0, height));
    height
}

fn notice<'a>(state: &'a crate::app::agents::State, paint: &Paint) -> &'a str {
    if state.page == Page::Install {
        return paint.hint(Message::ProvidersNextInstall);
    }
    if state.failure(state.selected).is_some() {
        if state.page == Page::Providers {
            return diagnostic_reason(state, paint);
        }
        if matches!(state.page, Page::Actions | Page::Diagnostics) {
            // The fixed failure summary owns these pages, not an older setup/auth toast.
            return "";
        }
    }
    if let Some(outcome) = state.auth_outcome(state.selected) {
        return paint.hint(match outcome {
            crate::providers::auth::Outcome::Authenticated => Message::ProvidersNextModel,
            crate::providers::auth::Outcome::Unknown => Message::ProvidersLoginUnknown,
            crate::providers::auth::Outcome::NotAuthenticated => Message::DiagnosticsAuthentication,
            crate::providers::auth::Outcome::Cancelled => Message::DiagnosticsLoginCancelled,
            crate::providers::auth::Outcome::Expired => Message::DiagnosticsLoginExpired,
            crate::providers::auth::Outcome::Failed => Message::DiagnosticsLoginFailed,
            crate::providers::auth::Outcome::Refused => Message::DiagnosticsLoginRefused,
        });
    }
    let message = match state.notice.as_str() {
        "" | "Enter chooses a ready provider; Right opens Actions and setup." => {
            Message::ProvidersNextSetup
        }
        "Private setup cancelled; no connection settings changed." => {
            Message::ProvidersSetupCancelled
        }
        "Login continuation expired; no connection settings changed." => {
            Message::ProvidersLoginStale
        }
        "Login did not complete. Retry the selected method."
        | "Login returned no usable credential; retry this method." => Message::ProvidersLoginRetry,
        "Preparing connection review; no provider program launched." => {
            Message::ProvidersPreparingReview
        }
        "Connection review cancelled; no provider program launched." => {
            Message::ProvidersReviewCancelled
        }
        "Refreshing provider configuration; no program or network request launched." => {
            Message::ProvidersRefreshing
        }
        "Blank key keeps source. Ctrl+U clears a field; Ctrl+S saves locally without model metadata." => {
            Message::ProvidersPrivateEditing
        }
        "Login saved; choose model metadata only. No task sent." => Message::ProvidersLoginSaved,
        _ => return &state.notice,
    };
    paint.hint(message)
}

/// Byte-safe viewport whose caret and clipping are measured in terminal cells.
fn field_view(value: &str, cursor: usize, capacity: usize) -> (&str, usize) {
    let capacity = capacity.max(1);
    let cursor = value
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(value.len()))
        .take_while(|index| *index <= cursor)
        .last()
        .unwrap_or(0);
    let mut start = cursor;
    let mut before = 0;
    for (index, character) in value[..cursor].char_indices().rev() {
        let width = ratatui::text::Span::raw(&value[index..index + character.len_utf8()]).width();
        if before + width >= capacity {
            break;
        }
        start = index;
        before += width;
    }
    let mut end = start;
    let mut width = 0;
    for (index, character) in value[start..].char_indices() {
        let cells =
            ratatui::text::Span::raw(&value[start + index..start + index + character.len_utf8()])
                .width();
        if width + cells > capacity {
            break;
        }
        width += cells;
        end = start + index + character.len_utf8();
    }
    (&value[start..end], before)
}

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
        Method::OpenAiBrowser => Message::ProvidersAgentsBrowserLogin,
        Method::OpenAiDevice => Message::ProvidersAgentsDeviceLogin,
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
        "Choose OpenAI connection method" => Message::ProvidersFamilyMethods,
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
                    let name = if crate::app::agents::openai_family(index) {
                        "OpenAI"
                    } else {
                        agent.map_or(state.profiles[index].name.as_str(), |agent| agent.label())
                    };
                    text(
                        frame,
                        row(body, 1 + offset as u16, 1),
                        &format!(
                            "{}{} {} | {}",
                            if index == state.selected { ">" } else { " " },
                            if index == state.active
                                || crate::app::agents::openai_family(index)
                                    && crate::app::agents::openai_family(state.active)
                            {
                                "*"
                            } else {
                                " "
                            },
                            name,
                            if crate::app::agents::openai_family(index)
                                && state.readiness(index)
                                    == crate::app::agents::Readiness::MissingCli
                            {
                                paint.hint(Message::ProvidersStatusSetup)
                            } else {
                                readiness_label(state, index, paint)
                            }
                        ),
                        if index == state.selected {
                            paint.accent
                        } else {
                            paint.surface
                        },
                    );
                }
            }
            Page::OpenAi | Page::Account => {
                let account = state.page == Page::Account;
                text(
                    frame,
                    row(body, 0, 1),
                    if account {
                        paint.hint(Message::ProvidersAccountBoundary)
                    } else {
                        "OpenAI"
                    },
                    paint.accent,
                );
                let labels: &[Message] = if account {
                    &[
                        Message::ProvidersAgentsBrowserLogin,
                        Message::ProvidersAgentsDeviceLogin,
                        Message::ProvidersAgentsStorage,
                        Message::ProvidersFamilyAdvanced,
                    ]
                } else if state.family_has_current() {
                    &[
                        Message::ProvidersFamilyAccount,
                        Message::ProvidersFamilyApi,
                        Message::ProvidersFamilyCustom,
                        Message::ProvidersFamilyCli,
                        Message::ProvidersFamilyCurrent,
                    ]
                } else {
                    &[
                        Message::ProvidersFamilyAccount,
                        Message::ProvidersFamilyApi,
                        Message::ProvidersFamilyCustom,
                        Message::ProvidersFamilyCli,
                    ]
                };
                for (index, &label) in labels.iter().enumerate() {
                    let label = paint.hint(label);
                    let value = if account && index == 2 {
                        format!(
                            "{label}: {}",
                            paint.hint(if state.login_persist {
                                Message::ProvidersAgentsSavePrivateLocalPlaintext
                            } else {
                                Message::ProvidersAgentsSessionOlderSavedProfileUnchanged
                            })
                        )
                    } else if !account && index == 4 {
                        format!(
                            "{label} · {}",
                            crate::providers::Agent::from_index(state.selected)
                                .map_or("", |agent| agent
                                    .command()
                                    .trim_start_matches("provider-"))
                        )
                    } else {
                        label.to_owned()
                    };
                    text(
                        frame,
                        row(body, 1 + index as u16, 1),
                        &format!(
                            "{} {value}",
                            if state.family_choice == index {
                                ">"
                            } else {
                                " "
                            }
                        ),
                        if state.family_choice == index {
                            paint.accent
                        } else {
                            paint.surface
                        },
                    );
                }
                text(
                    frame,
                    row(body, 6, body.height.saturating_sub(6)),
                    if account {
                        paint.hint(Message::ProvidersAccountReplace)
                    } else {
                        paint.hint(Message::ProvidersFamilyBoundary)
                    },
                    paint.muted,
                );
            }
            Page::Install => {
                state.scroll_max = selection_details(
                    frame,
                    body,
                    Paragraph::new(setup_hint(state.selected, paint))
                        .style(paint.surface)
                        .wrap(Wrap { trim: false }),
                    state.scroll,
                );
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
                        readiness_label(state, state.selected, paint)
                    ),
                    paint.accent,
                );
                let summary_height = diagnostic_summary(
                    frame,
                    row(body, 1, body.height.saturating_sub(1)),
                    state,
                    paint,
                );
                let actions = state.actions();
                let capacity = usize::from(body.height.saturating_sub(1 + summary_height)).max(1);
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
                        row(body, 1 + summary_height + offset as u16, 1),
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
                let summary_height = diagnostic_summary(frame, body, state, paint);
                let profile_details = format!(
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
                let mut value = String::new();
                value.push_str(paint.hint(Message::DiagnosticsDetailBoundary));
                if let Some(failure) = state.failure(state.selected) {
                    value.push('\n');
                    value.push_str(&match failure {
                        crate::providers::diagnostics::Failure::Http(status) => paint.format(
                            Template::DiagnosticsDetailHttp,
                            &[("status", &status.to_string())],
                        ),
                        _ => paint.format(
                            Template::DiagnosticsDetailLocal,
                            &[("category", diagnostic_reason(state, paint))],
                        ),
                    });
                }
                if let Some(detail) = state.preparation_detail(state.selected) {
                    value.push('\n');
                    value.push_str(&paint.format(
                        Template::DiagnosticsDetailPreparation,
                        &[("detail", detail)],
                    ));
                }
                value.push('\n');
                value.push_str(&profile_details);
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
                    row(
                        body,
                        summary_height,
                        body.height.saturating_sub(summary_height),
                    ),
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
            &crate::workspace::privacy::redact(notice(state, paint)),
            paint.muted,
        );
        let footer = match state.page {
            Page::OpenAi => paint.hint(Message::ProvidersFamilyNavigation).into(),
            Page::Account => paint.hint(Message::DiagnosticsAccountNavigation).into(),
            Page::Install => paint.hint(Message::ProvidersNextInstall).into(),
            Page::Providers => paint.format(
                Template::ProvidersAgentsTypeSearchUpDownSelectEnterRight,
                &[("action", (action_hint(paint, state.enter_hint())))],
            ),
            Page::Methods => paint.hint(Message::DiagnosticsMethodNavigation).into(),
            Page::Actions => paint.hint(Message::DiagnosticsActionsNavigation).into(),
            Page::Diagnostics => paint.hint(Message::DiagnosticsNavigation).into(),
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
        let native_summary;
        let summary = if let Some(plan) = &state.auth_preview {
            if matches!(
                plan.method,
                crate::providers::auth::Method::OpenAiBrowser
                    | crate::providers::auth::Method::OpenAiDevice
            ) {
                native_summary = vec![
                    format!("OpenAI / ChatGPT · {}", method_label(plan.method, paint)),
                    paint.hint(Message::ProvidersAccountBoundary).into(),
                    paint.hint(Message::ProvidersAccountEndpoint).into(),
                    paint.hint(Message::ProvidersAccountReplace).into(),
                    paint
                        .hint(if state.openai_login_persist() {
                            Message::ProvidersAccountPrivate
                        } else {
                            Message::ProvidersAccountSession
                        })
                        .into(),
                    paint.hint(Message::ProvidersAccountPlaintext).into(),
                    paint.hint(Message::ProvidersAccountContinue).into(),
                    paint.hint(Message::ProvidersAccountExpiry).into(),
                ];
                &native_summary
            } else {
                &plan.summary
            }
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
    let notice = notice(state, paint).to_owned();
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
    let disclosure = paint.hint(Message::ProvidersAgentsSaveModelSelectRouteMetadataNeverSend);
    let disclosure_fits = disclosure.lines().count() <= 2
        && disclosure
            .lines()
            .all(|line| ratatui::text::Span::raw(line).width() <= usize::from(content.width));
    editor.visible = room && disclosure_fits;
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
            ConnectionField::Advanced => (
                paint.hint(Message::ProvidersFamilyAdvanced),
                if editor.advanced { "-" } else { "+" }.into(),
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
        let (displayed, caret) = if private {
            (value.as_ref(), 0)
        } else {
            field_view(
                &value,
                if editor.field == index {
                    editor.cursor
                } else {
                    0
                },
                capacity,
            )
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
                caret
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
    // Two fixed rows; saving is disabled unless every disclosure cell fits.
    text(
        frame,
        row(content, content.height.saturating_sub(5), 2),
        disclosure,
        paint.muted,
    );
    state.scroll_max = selection_details(
        frame,
        row(content, content.height.saturating_sub(3), 2),
        Paragraph::new(crate::workspace::privacy::redact(&notice))
            .style(paint.muted)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll = state.scroll.min(state.scroll_max);
    text(
        frame,
        row(content, content.height.saturating_sub(1), 1),
        if editor.visible {
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
    fn minimum_diagnostic_summary_keeps_complete_reason_and_next_step_in_both_languages() {
        use crate::providers::{Agent, diagnostics::Failure};
        for language in ["en", "ko"] {
            let language = crate::i18n::Language::parse(language).unwrap();
            let style = Style::default();
            let paint = Paint {
                language,
                base: style,
                surface: style,
                muted: style,
                border: style,
                accent: style,
                selected: style,
            };
            for width in [54, 114] {
                let mut app = App::new("/unused".into(), width + 6, 18);
                app.agents.selected = Agent::Compatible.index();
                for failure in [
                    Failure::Http(401),
                    Failure::Http(403),
                    Failure::Http(404),
                    Failure::Http(429),
                    Failure::Transport,
                    Failure::Incompatible,
                    Failure::VersionCheck,
                    Failure::LoginCancelled,
                    Failure::LoginExpired,
                    Failure::LoginFailed,
                    Failure::LoginRefused,
                ] {
                    app.agents.connection_failed(Agent::Compatible, failure);
                    let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
                    terminal
                        .draw(|frame| {
                            let height =
                                diagnostic_summary(frame, frame.area(), &app.agents, &paint);
                            assert!(
                                height <= 6,
                                "reason/next must leave inspectable detail rows"
                            );
                        })
                        .unwrap();
                    let buffer = terminal.backend().buffer();
                    let cells: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                    let visible: String = cells.chars().filter(|c| !c.is_whitespace()).collect();
                    for fact in [
                        diagnostic_reason(&app.agents, &paint),
                        diagnostic_next(&app.agents, &paint),
                    ] {
                        let complete: String =
                            fact.chars().filter(|c| !c.is_whitespace()).collect();
                        assert!(
                            visible.contains(&complete),
                            "complete diagnostic fact must remain visible"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn full_connection_sheet_displays_every_disclosure_cell_in_both_languages() {
        for language in ["en", "ko"] {
            let language = crate::i18n::Language::parse(language).unwrap();
            let config = Config {
                ascii: true,
                language,
                no_animation: true,
                animation_override: Some(false),
                theme: crate::config::Theme::Dark,
                color: crate::config::ColorMode::Mono,
                images: crate::config::ImageMode::Off,
                assets_dir: std::path::PathBuf::new(),
                layout: crate::config::LayoutMode::Linear,
                layout_explicit: true,
                data_dir: std::path::PathBuf::new(),
                storage_policy: crate::store::Policy::default(),
                context_exclusions: vec![],
                capture_capacity: 1024,
            };
            let style = Style::default();
            let paint = Paint {
                language,
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
                    crate::providers::Agent::OpenAI,
                    crate::providers::Agent::Compatible,
                ] {
                    let mut app = App::new("/unused".into(), width, height);
                    app.agents.selected = agent.index();
                    app.agents.connection = Some(crate::app::agents::ConnectionEditor {
                        form: crate::providers::direct::config::Setup {
                            credential: String::new(),
                            base_url: "https://api.example/v1".into(),
                            model: "한글-model".into(),
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
                        advanced: true,
                    });
                    app.agents.paste_connection("private-credential-sentinel");
                    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    for save_selected in [false, true] {
                        if save_selected {
                            app.agents.connection.as_mut().unwrap().field = app
                                .agents
                                .connection_fields()
                                .iter()
                                .position(|field| *field == ConnectionField::Save)
                                .unwrap();
                        }
                        terminal
                            .draw(|frame| render(frame, frame.area(), &mut app, &config, &paint))
                            .unwrap();
                        assert!(app.agents.connection.as_ref().unwrap().visible);
                        let buffer = terminal.backend().buffer();
                        let display: String =
                            buffer.content.iter().map(|cell| cell.symbol()).collect();
                        assert!(!display.contains("private-credential-sentinel"));
                        assert!(!display.contains("private-account-sentinel"));
                        let outer = sheet(Rect::new(0, 0, width, height), 104, height - 2);
                        // Use the panel's content bounds, including horizontal padding.
                        let content = panel(
                            paint.hint(Message::ProvidersAgentsConnections),
                            &config,
                            &paint,
                            true,
                        )
                        .inner(outer);
                        let disclosure = paint
                            .hint(Message::ProvidersAgentsSaveModelSelectRouteMetadataNeverSend);
                        for (line, value) in disclosure.lines().enumerate() {
                            let mut x = content.x;
                            let y = content.y + content.height - 5 + line as u16;
                            for character in value.chars() {
                                let glyph = character.to_string();
                                assert_eq!(
                                    buffer[(x, y)].symbol(),
                                    glyph,
                                    "{language:?} {width}x{height} {agent:?}, save_selected={save_selected}, disclosure cell ({x}, {y})"
                                );
                                x += ratatui::text::Span::raw(glyph).width() as u16;
                            }
                            assert!(x <= content.right());
                        }
                    }
                    terminal = Terminal::new(TestBackend::new(52, 16)).unwrap();
                    app.width = 52;
                    app.height = 16;
                    terminal
                        .draw(|frame| render(frame, frame.area(), &mut app, &config, &paint))
                        .unwrap();
                    assert!(!app.agents.connection.as_ref().unwrap().visible);
                }
            }
        }
    }

    #[test]
    fn private_unicode_viewport_keeps_utf8_boundaries_and_cell_caret() {
        assert_eq!(field_view("한글abc", 6, 5), ("한글a", 4));
        assert_eq!(field_view("한글abc", 9, 5), ("abc", 3));
        assert_eq!(field_view("한글abc", 3, 4), ("한글", 2));
        let (value, caret) = field_view("a한글b", 7, 4);
        assert_eq!((value, caret), ("글b", 2));
    }
}
