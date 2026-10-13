use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Clear, Paragraph, Wrap},
};

use super::{Paint, centered, clipped, panel, row, selection_details, text};
use crate::{app::App, config::Config};

pub(super) fn render(frame: &mut Frame, area: Rect, app: &mut App, config: &Config, paint: &Paint) {
    let area = centered(area, 104, area.height.saturating_sub(2));
    frame.render_widget(Clear, area);
    let auth_room = app.width >= 80 && app.height >= 24;
    let state = &mut app.agents;
    if state.auth_preview.is_some() && !auth_room {
        state.reviewed = false;
        state.affirmative = false;
    }
    let block = panel(
        if state.auth_preview.is_some() {
            " Explicit authentication / no task replay "
        } else if state.preview {
            " Trust executable / version probe only "
        } else {
            " Agent connections / no model requests "
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
            "Passive discovery pending. No executable is running.\nEsc Back / local workflow remains available",
            paint.surface,
        );
        return;
    };
    let header = format!(
        "{}/{} {}{}",
        state.selected + 1,
        state.profiles.len(),
        profile.name,
        if state.busy() { " / working" } else { "" }
    );
    text(
        frame,
        row(content, 0, 1),
        &clipped(&header, content.width as usize),
        paint.accent,
    );
    let mut value = String::new();
    let executable_debug = format!("{:?}", profile.executable);
    let executable = crate::privacy::redact(&executable_debug);
    if let Some(plan) = &state.auth_preview {
        value.push_str(&plan.lines.join("\n\n"));
    } else if state.preview {
        value.push_str("Trust applies to ONE version probe, not a session. Even --version executes arbitrary program code. No sandbox or safety guarantee is implied.\n\n");
        value.push_str(&format!(
            "Executable: {}\nLiteral argv: {:?}\nConfiguration / effects: {}\n\n",
            executable, profile.argv, profile.configuration
        ));
        for line in &profile.trust_preview {
            value.push_str(line);
            value.push('\n');
        }
        value.push_str("\nNo login, model request, adapter launch, or Eevee configuration/credential mutation is authorized. Executable/config identity is revalidated before launch; changes require a new preview. Provider code can have its own effects. Cancel if you do not trust this executable and its configuration.\n\nEND OF TRUST PREVIEW. Default is Stay. Select Trust explicitly only after reviewing these effects.");
    } else {
        value.push_str(&format!("LAST CONNECTION RESULT: {}\n\n", state.notice));
        if let Some(method) = crate::auth::methods(state.selected).get(state.method) {
            value.push_str(&format!(
                "Selected login method: {} (m changes method; l reviews it)\n\n",
                method.label()
            ));
        } else if state.selected == 1 {
            value.push_str(crate::auth::CLAUDE_POLICY);
            value.push_str("\n\n");
        }
        value.push_str(&format!(
            "Installed: {}\nExecutable: {}\nLiteral argv: {:?}\nVersion / compatibility: {}\nAuthentication: {}\nBilling source: {}\nConfiguration / trust scope: {}\nOS: {} / architecture: {}\nAdapter: {}\nAction readiness: {}\nProfile contract revision: {}\n\nPREFLIGHT ONLY: e separately reviews the selected provider/action. Each adapter needs its own pinned runtime, authentication and negotiated capabilities. Installed does not mean authenticated or supported.\n\n",
            profile.installed, executable, profile.argv, profile.version,
            profile.authentication, profile.billing, profile.configuration, profile.os,
            profile.architecture, profile.adapter, profile.action, profile.revision,
        ));
        for blocker in &profile.blockers {
            value.push_str(&format!("Blocker / remedy: {blocker}\n"));
        }
        value.push_str("\nPER-ACTION BOUNDARIES / EVIDENCE\n");
        for capability in &profile.capabilities {
            value.push_str(&format!(
                "{}: {}\nEvidence: {}\nRestriction / blocker: {}\n\n",
                capability.action, capability.boundary, capability.evidence, capability.blocker
            ));
        }
        value.push_str("Eevee-mediated controls, agent-policy controls and observed/external behavior are distinct. A cwd or permission prompt is not OS isolation. l separately reviews authentication; s reviews official status where available. Login never resumes a model request.\n\n");
        value.push_str(&state.notice);
    }
    let details = row(content, 1, content.height.saturating_sub(4));
    state.scroll_max = selection_details(
        frame,
        details,
        Paragraph::new(value)
            .style(paint.surface)
            .wrap(Wrap { trim: false }),
        state.scroll,
    );
    state.scroll = state.scroll.min(state.scroll_max);
    if (state.preview || state.auth_preview.is_some())
        && (state.auth_preview.is_none() || auth_room)
        && details.width >= 40
        && details.height >= 3
        && state.scroll >= state.scroll_max
    {
        state.reviewed = true;
    }
    let footer = if state.auth_preview.is_some() {
        if !auth_room {
            "Authentication approval requires at least 80 columns x 24 rows.\nResize to review / Esc Cancel / no launch"
        } else if !state.reviewed {
            "[Stay] / Authorize locked until end of review\nUp/Down or PgDn Review / Esc Cancel"
        } else if state.affirmative {
            "Stay / [Authorize this auth operation]\nTab Choose / Enter Confirm / Esc Cancel"
        } else {
            "[Stay] / Authorize this auth operation\nTab Choose / Enter Stay / Esc Cancel"
        }
    } else if state.preview {
        if !state.reviewed {
            "[Stay] / Trust locked until end of preview\nUp/Down or PgDn Review / Esc Cancel"
        } else if state.affirmative {
            "Stay / [Trust this version probe]\nTab Choose / Enter Confirm / Esc Cancel"
        } else {
            "[Stay] / Trust this version probe\nTab Choose / Enter Stay / Esc Cancel"
        }
    } else {
        "1-4 Profile / Enter Probe / m Method / l Login / s Status\nr Refresh / PgUp/Dn Scroll / Esc Back / no task replay"
    };
    text(
        frame,
        row(content, content.height.saturating_sub(3), 3),
        footer,
        paint.accent,
    );
}
