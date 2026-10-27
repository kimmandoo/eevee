//! Disclosure only: this table never grants tools, resumes sessions or dispatches work.
use super::{Agent, ModelCatalogue, ModelSelection, Operation};

#[derive(Clone, Copy)]
pub struct Route {
    pub short: &'static str,
    pub transport: &'static str,
    pub effort: &'static str,
    pub effort_control: bool,
    pub todo: &'static str,
    pub approvals: &'static str,
    pub session: &'static str,
    pub artifacts: &'static str,
}

const NATIVE_APPROVALS: &str = "exact file/command approval; no inherited grants";
const NATIVE_SESSION: &str = "unsupported: provider-owned resume; process-local public history only; use /new or send a follow-up";
const ACP_APPROVALS: &str =
    "exact client file/command approval; provider-owned tools remain external policy";
const ACP_SESSION: &str =
    "peer-advertised listing/resume only; unknown until inspected; /new remains available";
const ACP_ARTIFACTS: &str = "unsupported: provider-owned output retrieval; inspect /response or /artifacts for owned local command output";

pub const ROUTES: [Route; 10] = [
    Route {
        short: "Codex/CLI",
        transport: "controlled Codex app-server",
        effort: "requested, not acknowledged unless reported",
        effort_control: true,
        todo: "provider plan proposal; explicit adoption required",
        approvals: NATIVE_APPROVALS,
        session: "controlled Implement requires a fresh thread; public context may continue, never inherited authority; Plan resume remains separately inspected",
        artifacts: "owned immutable output via /artifacts",
    },
    Route {
        short: "Claude/SDK",
        transport: "Claude Agent SDK",
        effort: "requested, not acknowledged",
        effort_control: true,
        todo: "SDK todo proposal; explicit adoption required",
        approvals: "exact client file/command approval; SDK hooks/configuration remain disclosed external policy",
        session: "same-task owned SDK session only; /new always available",
        artifacts: "unsupported: SDK-owned output retrieval; inspect /response or /artifacts for owned local command output",
    },
    Route {
        short: "OpenCode/ACP",
        transport: "OpenCode ACP",
        effort: "requested, not acknowledged unless reported",
        effort_control: true,
        todo: "ACP plan proposal; explicit adoption required",
        approvals: ACP_APPROVALS,
        session: ACP_SESSION,
        artifacts: ACP_ARTIFACTS,
    },
    Route {
        short: "Gemini/ACP",
        transport: "Gemini ACP",
        effort: "requested, not acknowledged unless reported",
        effort_control: true,
        todo: "ACP plan proposal; explicit adoption required",
        approvals: ACP_APPROVALS,
        session: ACP_SESSION,
        artifacts: ACP_ARTIFACTS,
    },
    Route {
        short: "Devin/ACP",
        transport: "Devin ACP",
        effort: "requested, not acknowledged unless reported",
        effort_control: true,
        todo: "ACP plan proposal; explicit adoption required",
        approvals: ACP_APPROVALS,
        session: ACP_SESSION,
        artifacts: ACP_ARTIFACTS,
    },
    Route {
        short: "OpenAI/OAuth",
        transport: "native account Responses",
        effort: "requested, not acknowledged unless reported",
        effort_control: true,
        todo: "native todo tool proposal; explicit adoption required",
        approvals: NATIVE_APPROVALS,
        session: NATIVE_SESSION,
        artifacts: "owned immutable output via /artifacts",
    },
    Route {
        short: "OpenAI/API",
        transport: "native Responses",
        effort: "requested, not acknowledged unless reported",
        effort_control: true,
        todo: "native todo tool proposal; explicit adoption required",
        approvals: NATIVE_APPROVALS,
        session: NATIVE_SESSION,
        artifacts: "owned immutable output via /artifacts",
    },
    Route {
        short: "Anthropic",
        transport: "native Messages",
        effort: "effort fixed by provider; not applied",
        effort_control: false,
        todo: "native todo tool proposal; explicit adoption required",
        approvals: NATIVE_APPROVALS,
        session: NATIVE_SESSION,
        artifacts: "owned immutable output via /artifacts",
    },
    Route {
        short: "Gemini/API",
        transport: "native generateContent",
        effort: "effort fixed by provider; not applied",
        effort_control: false,
        todo: "native todo tool proposal; explicit adoption required",
        approvals: NATIVE_APPROVALS,
        session: NATIVE_SESSION,
        artifacts: "owned immutable output via /artifacts",
    },
    Route {
        short: "Custom",
        transport: "native configured Responses / Chat Completions",
        effort: "requested, not acknowledged unless reported",
        effort_control: true,
        todo: "native todo tool proposal; explicit adoption required",
        approvals: NATIVE_APPROVALS,
        session: NATIVE_SESSION,
        artifacts: "owned immutable output via /artifacts",
    },
];

pub fn route(agent: Agent) -> &'static Route {
    &ROUTES[agent as usize]
}

/// Stable order and keys for every route/model, including uninspected exact IDs.
pub const JOURNEY: [(&str, &str); 13] = [
    ("Connect", "/connect"),
    ("Model / effort", "/model · /effort"),
    ("Write Request", "Enter edit · Ctrl+S save"),
    ("Preview", "/evolution-preview"),
    ("Explicit send", "Enter Send · review then y when required"),
    ("Stream", "/response"),
    (
        "File / command approvals",
        "Enter review · choose exact decision",
    ),
    ("Todo progress", "/todo"),
    ("Follow-up / continue", "write Request · Enter Send"),
    ("Cancel", "/cancel"),
    ("Resume / new session", "/resume · /new"),
    ("Handoff", "/handoff"),
    ("Receipt", "/receipts"),
];

pub fn model_disclosure(
    catalogue: Option<&ModelCatalogue>,
    selection: &ModelSelection,
) -> &'static str {
    let model = catalogue.and_then(|catalogue| {
        selection
            .model
            .as_ref()
            .or(catalogue.current.as_ref())
            .and_then(|id| catalogue.models.iter().find(|model| &model.id == id))
    });
    match model {
        None => {
            "unknown model capabilities; exact ID retained; inspect /model, or explicitly send without assuming support"
        }
        Some(model) if model.efforts.is_empty() => {
            "effort fixed by provider; not applied; model tool/plan support unknown until reported"
        }
        Some(_) => "advertised effort ladder; model tool/plan support unknown until reported",
    }
}

pub fn disclosures(
    agent: Agent,
    operation: Operation,
    catalogue: Option<&ModelCatalogue>,
    selection: &ModelSelection,
) -> [String; 9] {
    let route = route(agent);
    let disclose = |value: &str| {
        value.strip_prefix("unsupported: ").map_or_else(
            || value.to_owned(),
            |reason| format!("unsupported on {}: {reason}", agent.label()),
        )
    };
    [
        format!("Route: {} / {}", agent.label(), route.transport),
        format!(
            "Model: {} / {}",
            selection.model.as_deref().unwrap_or("provider default"),
            model_disclosure(catalogue, selection)
        ),
        format!(
            "Effort: {} / {}",
            selection.effort.as_deref().unwrap_or("provider default"),
            route.effort
        ),
        format!("Approvals: {}", route.approvals),
        format!(
            "Todo: {} / no step list from this route until observed",
            route.todo
        ),
        format!(
            "Session: {}{}",
            disclose(route.session),
            if operation == Operation::Review {
                "; Review always needs fresh evidence/session"
            } else {
                ""
            }
        ),
        format!("Artifacts: {}", disclose(route.artifacts)),
        "Usage: unknown unless reported; unknown is not zero".into(),
        "Outcome: unknown until reported; provider claims are not verification".into(),
    ]
}
