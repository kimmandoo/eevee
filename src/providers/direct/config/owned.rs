//! Explicit native configuration, kept outside tasks, journals and provider-owned stores.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

mod storage;

const ERROR: &str = "Private storage refused: use a non-project, private user home without links/reparse points; or enter a new key for session-only.";
static SESSION: LazyLock<Mutex<HashMap<Agent, Stored>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Stored {
    base_url: String,
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    effort_model: Option<String>,
    protocol: String,
    secret: Option<String>,
    account_id: Option<String>,
    keyless: bool,
}
impl std::fmt::Debug for Stored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Stored { private configuration redacted }")
    }
}

/// Private form data: never put this in Request, a general editor or a journal.
pub(crate) struct Setup {
    pub credential: String,
    pub base_url: String,
    pub model: String,
    pub effort: Option<String>,
    pub effort_model: Option<String>,
    pub account_id: String,
    pub responses: bool,
    pub keyless: bool,
    pub persist: bool,
    pub source_notice: Option<String>,
}
impl std::fmt::Debug for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Setup { private configuration redacted }")
    }
}

impl Setup {
    /// An inherited default is meaningful only for its frozen configured model.
    pub fn effort_resets(&self) -> bool {
        self.effort.is_some()
            && self.effort_model.as_deref()
                != (!self.model.is_empty()).then_some(self.model.as_str())
    }
}

fn name(provider: Agent) -> &'static str {
    match provider {
        Agent::ChatGPT => "chatgpt.json",
        Agent::OpenAI => "openai.json",
        Agent::Anthropic => "anthropic.json",
        Agent::Google => "google.json",
        Agent::Compatible => "compatible.json",
        _ => "unsupported.json",
    }
}

pub(super) fn available<F: Fn(&str) -> Option<String>>(provider: Agent, input: &Inputs<F>) -> bool {
    if SESSION
        .lock()
        .is_ok_and(|entries| entries.contains_key(&provider))
    {
        return true;
    }
    // Passive discovery checks the source only, never credential bytes.
    input.home.as_ref().is_some_and(|home| {
        fs::symlink_metadata(home.join(".eevee-connections").join(name(provider)))
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= FILE_LIMIT)
    })
}

pub(super) fn load<F: Fn(&str) -> Option<String>>(
    provider: Agent,
    input: &Inputs<F>,
) -> Result<Option<Stored>, String> {
    if !provider.is_direct() {
        return Ok(None);
    }
    if let Some(entry) = SESSION
        .lock()
        .map_err(|_| ERROR.to_string())?
        .get(&provider)
        .cloned()
    {
        return Ok(Some(entry));
    }
    let Some(home) = input.home.as_deref() else {
        return Ok(None);
    };
    let directory = home.join(".eevee-connections");
    if fs::symlink_metadata(&directory)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(None);
    }
    let store = storage::Directory::open(home, input.project.as_deref(), false)?;
    let Some(mut file) = store.read(name(provider))? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.by_ref()
        .take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR.to_string())?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(ERROR.into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|_| "Private native configuration is invalid; replace it using connection setup. No stored contents are displayed.".into())
}

fn protocol(provider: Agent, value: &str) -> Result<Protocol, String> {
    match (provider, value) {
        (Agent::ChatGPT, "codex") => Ok(Protocol::Codex),
        (Agent::OpenAI, "responses") => Ok(Protocol::Responses),
        (Agent::Anthropic, "messages") => Ok(Protocol::Messages),
        (Agent::Google, "gemini") => Ok(Protocol::Gemini),
        (Agent::Compatible, "responses") => Ok(Protocol::Responses),
        (Agent::Compatible, "chat_completions") => Ok(Protocol::ChatCompletions),
        _ => Err("Protocol does not belong to the selected native route.".into()),
    }
}

pub(super) fn connection(provider: Agent, entry: Stored, now: u64) -> Result<Connection, String> {
    let protocol = protocol(provider, &entry.protocol)?;
    let base_url = validate_base(&entry.base_url)?;
    if provider == Agent::ChatGPT && base_url != CODEX_BASE {
        return Err("ChatGPT subscription access tokens are scoped to the official Codex backend; choose a separate API route for custom endpoints.".into());
    }
    validate_model(entry.model.as_deref())?;
    validate_model(entry.effort.as_deref())?;
    validate_model(entry.effort_model.as_deref())?;
    if entry.effort.is_some() && (provider != Agent::ChatGPT || entry.effort_model != entry.model)
        || entry.effort.is_none() && entry.effort_model.is_some()
    {
        return Err("Configured reasoning effort has no matching model association. Review setup; changing model explicitly resets the old effort default.".into());
    }
    if entry.keyless && provider != Agent::Compatible {
        return Err("Only the Compatible route supports explicit keyless mode.".into());
    }
    let secret = if entry.keyless {
        None
    } else {
        let secret = entry.secret.filter(|value| valid_secret(value)).ok_or_else(|| "Enter a nonempty native API key/access token, or explicitly choose Compatible keyless mode.".to_string())?;
        if provider == Agent::Anthropic && secret.starts_with("sk-ant-oat") {
            return Err("Anthropic subscription OAuth tokens are not API keys. Use an Anthropic API key; subscription login remains a separate official route.".into());
        }
        Some(secret)
    };
    let claims = if provider == Agent::ChatGPT {
        token_claims(secret.as_deref().unwrap_or_default(), now)?
    } else {
        Claims {
            account_id: None,
            residency: None,
        }
    };
    let account_id = entry.account_id.or(claims.account_id);
    if account_id
        .as_ref()
        .is_some_and(|value| !valid_secret(value))
        || claims
            .residency
            .as_ref()
            .is_some_and(|value| !valid_secret(value))
    {
        return Err(
            "Native account metadata is invalid; replace it in private connection setup.".into(),
        );
    }
    if provider != Agent::ChatGPT && account_id.is_some() {
        return Err("Account metadata is available only for the ChatGPT native route.".into());
    }
    Ok(Connection {
        protocol,
        base_url,
        model: entry.model,
        effort: entry.effort,
        provider,
        secret,
        account_id,
        residency: claims.residency,
        description: format!(
            "{}: {:?}; explicit own native configuration; {}; configured only, not authenticated. Native selected-path Implement supports Eevee-mediated create/replace after exact one-shot approval. Unscoped Implement, Plan, Review and metadata do not write files. No shell, hooks, MCP or automatic edits. No automatic renewal, retry or account rotation.",
            provider.label(),
            protocol,
            if entry.keyless {
                "keyless: no authentication header"
            } else {
                "credential scoped to reviewed endpoint"
            }
        ),
    })
}

fn validate_model(value: Option<&str>) -> Result<(), String> {
    if value.is_some_and(|value| value.is_empty() || value.len() > 256
        || crate::workspace::privacy::redact(value) != value
        || value.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))) {
        return Err("Model/effort defaults must be bounded identifiers without secrets or control characters.".into());
    }
    Ok(())
}

pub(crate) fn setup(provider: Agent) -> Result<Setup, String> {
    let input = inputs();
    let (own, source_notice) = match load(provider, &input) {
        Ok(entry) => (entry, None),
        Err(_) => (None, Some("Saved source is unsafe/unreadable. Enter a new key or choose explicit keyless; session-only needs no storage.".into())),
    };
    setup_from(provider, &input, own, source_notice)
}

fn setup_from<F: Fn(&str) -> Option<String>>(
    provider: Agent,
    input: &Inputs<F>,
    own: Option<Stored>,
    mut source_notice: Option<String>,
) -> Result<Setup, String> {
    let (base_url, model, effort, effort_model, responses, keyless) = if let Some(entry) = own {
        (
            entry.base_url,
            entry.model,
            entry.effort,
            entry.effort_model,
            entry.protocol == "responses",
            entry.keyless,
        )
    } else {
        let settings = match settings(provider, input) {
            Ok(settings) => Ok(settings),
            Err(_) => {
                if provider == Agent::ChatGPT {
                    let warning = "Provider defaults unreadable: no model/effort imported. Enter explicit defaults; no provider file was repaired.";
                    if let Some(notice) = source_notice.as_mut() {
                        notice.push(' ');
                        notice.push_str(warning);
                    } else {
                        source_notice = Some(warning.into());
                    }
                }
                settings(
                    provider,
                    &Inputs {
                        get: |_: &str| None,
                        home: None,
                        config_home: None,
                        project: None,
                        now: input.now,
                    },
                )
            }
        };
        match settings {
            Ok(settings) => {
                let effort_model = if settings.effort.is_some() {
                    settings.model.clone()
                } else {
                    None
                };
                (
                    settings.base_url,
                    settings.model,
                    settings.effort,
                    effort_model,
                    settings.protocol == Protocol::Responses,
                    settings.keyless,
                )
            }
            Err(_) if provider == Agent::Compatible => {
                (String::new(), None, None, None, false, false)
            }
            Err(reason) => return Err(reason),
        }
    };
    Ok(Setup {
        credential: String::new(),
        base_url,
        model: model.unwrap_or_default(),
        effort,
        effort_model,
        account_id: String::new(),
        responses,
        keyless,
        persist: false,
        source_notice,
    })
}

/// Explicit user action: read only the official CLI source, never an environment
/// token, into the masked draft. Account selection still requires pressing Save.
pub(crate) fn load_cli_credential(form: &mut Setup) -> Result<(), String> {
    load_cli_with(form, inputs())
}

fn load_cli_with<F: Fn(&str) -> Option<String>>(
    form: &mut Setup,
    input: Inputs<F>,
) -> Result<(), String> {
    let Inputs {
        get,
        home,
        config_home,
        project,
        now,
    } = input;
    let source = Inputs {
        get: |name: &str| match name {
            "CODEX_ACCESS_TOKEN" | "CODEX_ACCOUNT_ID" => None,
            _ => get(name),
        },
        home,
        config_home,
        project,
        now,
    };
    let connection = resolve_with(Agent::ChatGPT, &source)?;
    form.credential = connection.secret.ok_or_else(|| "Official CLI source has no readable access token. Supply a token or complete official CLI login; keychain-only storage is unsupported.".to_string())?;
    form.account_id = connection.account_id.unwrap_or_default();
    form.base_url = CODEX_BASE.into();
    form.source_notice = None;
    Ok(())
}

pub(crate) struct Applied {
    pub changed: bool,
    pub durable: bool,
    pub effort_reset: bool,
}

/// Frozen local deletion target; it contains no credential bytes or task data.
pub(crate) struct ForgetPlan {
    provider: Agent,
    directory: Option<storage::Directory>,
    file: Option<storage::FileIdentity>,
    absent_directory: Option<PathBuf>,
    session: Option<String>,
}

impl std::fmt::Debug for ForgetPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ForgetPlan { exact private source identities redacted }")
    }
}

impl ForgetPlan {
    pub fn agent(&self) -> Agent {
        self.provider
    }

    pub fn summary(&self) -> Vec<String> {
        vec![
            format!("Forget app connection: {}", self.provider.label()),
            format!(
                "Remove this route's session override: {}",
                if self.session.is_some() {
                    "yes"
                } else {
                    "absent"
                }
            ),
            format!(
                "Own file: ~/.eevee-connections/{} ({})",
                name(self.provider),
                if self.file.is_some() {
                    "remove"
                } else {
                    "absent"
                }
            ),
            "Other routes, HOME/provider stores and tasks untouched.".into(),
            "No journal deletion, remote logout/revocation or network.".into(),
            "Future requests may use existing env/provider source.".into(),
            "Only reviewed own identities; changes require new review.".into(),
        ]
    }
}

fn entry_identity(entry: &Stored) -> String {
    let mut digest = Sha256::new();
    digest.update(b"eevee/own-connection/deletion/v1");
    digest.update([u8::from(entry.keyless)]);
    for part in [
        Some(entry.base_url.as_str()),
        entry.model.as_deref(),
        entry.effort.as_deref(),
        entry.effort_model.as_deref(),
        Some(entry.protocol.as_str()),
        entry.secret.as_deref(),
        entry.account_id.as_deref(),
    ] {
        digest.update([u8::from(part.is_some())]);
        let part = part.unwrap_or_default();
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

pub(crate) fn prepare_forget(provider: Agent) -> Result<ForgetPlan, String> {
    let session = SESSION
        .lock()
        .map_err(|_| ERROR.to_string())?
        .get(&provider)
        .map(entry_identity);
    prepare_forget_from(provider, &inputs(), session)
}

fn prepare_forget_from<F: Fn(&str) -> Option<String>>(
    provider: Agent,
    input: &Inputs<F>,
    session: Option<String>,
) -> Result<ForgetPlan, String> {
    if !provider.is_direct() {
        return Err("Only app-owned native connections can be forgotten here.".into());
    }
    let mut absent_directory = None;
    let directory = if let Some(home) = input.home.as_deref() {
        let path = home.join(".eevee-connections");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                absent_directory = Some(path);
                None
            }
            _ => Some(storage::Directory::open(
                home,
                input.project.as_deref(),
                false,
            )?),
        }
    } else {
        None
    };
    let file = directory
        .as_ref()
        .map(|directory| directory.identity(name(provider)))
        .transpose()?
        .flatten();
    if file.is_none() && session.is_none() {
        return Err("No app-owned override for this route. Environment/provider-owned sources were not changed.".into());
    }
    Ok(ForgetPlan {
        provider,
        directory,
        file,
        absent_directory,
        session,
    })
}

pub(crate) fn forget_connection(plan: ForgetPlan) -> Result<Applied, String> {
    let mut session = SESSION.lock().map_err(|_| ERROR.to_string())?;
    forget_from(plan, &mut session)
}

fn forget_from(plan: ForgetPlan, session: &mut HashMap<Agent, Stored>) -> Result<Applied, String> {
    let changed = || "App connection changed; review Forget again. No task was sent.".to_string();
    if session.get(&plan.provider).map(entry_identity) != plan.session {
        return Err(changed());
    }
    if let Some(path) = &plan.absent_directory
        && !fs::symlink_metadata(path)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return Err(changed());
    }
    let durable = if let Some(directory) = &plan.directory {
        match plan.file {
            Some(identity) => directory.remove(name(plan.provider), identity)?,
            None => {
                if directory.identity(name(plan.provider))?.is_some() {
                    return Err(changed());
                }
                true
            }
        }
    } else {
        true
    };
    let removed_session = session.remove(&plan.provider).is_some();
    Ok(Applied {
        changed: removed_session || plan.file.is_some(),
        durable,
        effort_reset: false,
    })
}

fn stored_from_setup(
    provider: Agent,
    form: &Setup,
    secret: Option<String>,
    account_id: Option<String>,
) -> Result<Stored, String> {
    let effort = if provider == Agent::ChatGPT && !form.effort_resets() {
        form.effort.clone()
    } else {
        None
    };
    Ok(Stored {
        base_url: form.base_url.clone(),
        model: (!form.model.is_empty()).then(|| form.model.clone()),
        effort_model: if effort.is_some() {
            form.effort_model.clone()
        } else {
            None
        },
        effort,
        protocol: match provider {
            Agent::ChatGPT => "codex",
            Agent::OpenAI => "responses",
            Agent::Anthropic => "messages",
            Agent::Google => "gemini",
            Agent::Compatible if form.responses => "responses",
            Agent::Compatible => "chat_completions",
            _ => return Err("Select a native connection route.".into()),
        }
        .into(),
        secret,
        account_id,
        keyless: form.keyless,
    })
}

/// Apply only from the explicit Save action with its storage/scope disclosure visible.
pub(crate) fn apply_setup(provider: Agent, form: &Setup) -> Result<Applied, String> {
    let input = inputs();
    let had_session = SESSION
        .lock()
        .map_err(|_| ERROR.to_string())?
        .contains_key(&provider);
    let previous = match load(provider, &input) {
        Ok(entry) => entry,
        Err(_) if !form.credential.is_empty() || form.keyless => None,
        Err(reason) => return Err(reason),
    };
    let fallback = if !form.keyless && form.credential.is_empty() && previous.is_none() {
        Some(resolve_with(provider, &input)?)
    } else {
        None
    };
    let secret = if form.keyless {
        None
    } else if !form.credential.is_empty() {
        Some(form.credential.clone())
    } else {
        previous
            .as_ref()
            .and_then(|entry| entry.secret.clone())
            .or_else(|| {
                fallback
                    .as_ref()
                    .and_then(|connection| connection.secret.clone())
            })
    };
    let account_id = if provider != Agent::ChatGPT {
        None
    } else if !form.account_id.is_empty() {
        Some(form.account_id.clone())
    } else if form.credential.is_empty() {
        previous
            .as_ref()
            .and_then(|entry| entry.account_id.clone())
            .or_else(|| {
                fallback
                    .as_ref()
                    .and_then(|connection| connection.account_id.clone())
            })
    } else {
        None
    };
    let entry = stored_from_setup(provider, form, secret, account_id)?;
    let next = connection(provider, entry.clone(), input.now)?;
    let changed = previous
        .and_then(|entry| connection(provider, entry, input.now).ok())
        .or(fallback)
        .is_none_or(|old| old.fingerprint() != next.fingerprint());
    let mut session = SESSION.lock().map_err(|_| ERROR.to_string())?;
    let mut durable = true;
    if form.persist {
        let home = input.home.as_deref().ok_or_else(|| ERROR.to_string())?;
        let store = storage::Directory::open(home, input.project.as_deref(), true)?;
        let bytes = serde_json::to_vec(&entry).map_err(|_| ERROR.to_string())?;
        if bytes.len() as u64 > FILE_LIMIT {
            return Err("Private native configuration exceeds its storage bound.".into());
        }
        durable = store.write(name(provider), &bytes)?;
        session.remove(&provider);
    } else {
        session.insert(provider, entry);
    }
    // Even identical settings change when explicitly moved between residency modes.
    Ok(Applied {
        changed: changed || form.persist == had_session,
        durable,
        effort_reset: form.effort_resets(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stored(provider: Agent, secret: &str) -> Stored {
        Stored {
            base_url: if provider == Agent::ChatGPT {
                CODEX_BASE
            } else {
                "https://api.example/v1"
            }
            .into(),
            model: Some("chosen".into()),
            effort: None,
            effort_model: None,
            protocol: if provider == Agent::ChatGPT {
                "codex"
            } else {
                "responses"
            }
            .into(),
            secret: Some(secret.into()),
            account_id: None,
            keyless: false,
        }
    }
    #[test]
    fn conversation_scope_tracks_identity_not_model_or_display() {
        let mut a = connection(Agent::OpenAI, stored(Agent::OpenAI, "fake-key-a"), 100).unwrap();
        let scope = a.conversation_scope();
        a.model = Some("another".into());
        a.effort = Some("high".into());
        a.description = "other wording".into();
        assert_eq!(scope, a.conversation_scope());
        a.secret = Some("fake-key-b".into());
        assert_ne!(scope, a.conversation_scope());
        a.secret = Some("fake-key-a".into());
        a.base_url.push_str("/other");
        assert_ne!(scope, a.conversation_scope());
        assert!(!format!("{a:?}").contains("fake-key"));
    }

    #[test]
    fn conversation_scope_separates_account_residency_protocol_and_provider() {
        let mut connection =
            connection(Agent::ChatGPT, stored(Agent::ChatGPT, "fake-token"), 100).unwrap();
        let scope = connection.conversation_scope();
        connection.account_id = Some("account-a".into());
        assert_ne!(scope, connection.conversation_scope());
        connection.account_id = None;
        connection.residency = Some("region-a".into());
        assert_ne!(scope, connection.conversation_scope());
        connection.residency = None;
        connection.protocol = Protocol::Responses;
        assert_ne!(scope, connection.conversation_scope());
        connection.protocol = Protocol::Codex;
        connection.provider = Agent::OpenAI;
        assert_ne!(scope, connection.conversation_scope());
    }

    #[test]
    fn explicit_cli_import_never_substitutes_environment_or_existing_private_draft() {
        let mut form = Setup {
            credential: "existing-draft".into(),
            base_url: CODEX_BASE.into(),
            model: String::new(),
            effort: None,
            effort_model: None,
            account_id: "existing-account".into(),
            responses: false,
            keyless: false,
            persist: false,
            source_notice: None,
        };
        let input = Inputs {
            get: |name: &str| match name {
                "CODEX_ACCESS_TOKEN" => Some("environment-token-must-not-be-imported".into()),
                "CODEX_ACCOUNT_ID" => Some("environment-account-must-not-be-imported".into()),
                _ => None,
            },
            home: None,
            config_home: None,
            project: None,
            now: 100,
        };
        assert!(load_cli_with(&mut form, input).is_err());
        assert_eq!(form.credential, "existing-draft");
        assert_eq!(form.account_id, "existing-account");
    }
    #[test]
    fn keyless_and_subscription_routes_cannot_be_confused() {
        let mut entry = stored(Agent::OpenAI, "fake");
        entry.keyless = true;
        assert!(connection(Agent::OpenAI, entry, 100).is_err());
        let mut entry = stored(Agent::ChatGPT, "fake");
        entry.base_url = "https://api.example/v1".into();
        assert!(connection(Agent::ChatGPT, entry, 100).is_err());
        let mut entry = stored(Agent::OpenAI, "sk-ant-oat-fake");
        entry.protocol = "messages".into();
        assert!(connection(Agent::Anthropic, entry, 100).is_err());
    }
    #[test]
    fn private_values_have_no_debug_representation() {
        let entry = stored(Agent::OpenAI, "private-sentinel");
        assert!(!format!("{entry:?}").contains("private-sentinel"));
        let setup = Setup {
            credential: "private-sentinel".into(),
            base_url: String::new(),
            model: String::new(),
            effort: None,
            effort_model: None,
            account_id: "private-account".into(),
            responses: false,
            keyless: false,
            persist: false,
            source_notice: None,
        };
        assert!(!format!("{setup:?}").contains("private-"));
    }

    #[test]
    fn saving_explicit_chatgpt_defaults_preserves_effort_and_its_model_binding() {
        let input = Inputs {
            get: |name: &str| match name {
                "CODEX_MODEL" => Some("explicit-model".into()),
                "CODEX_REASONING_EFFORT" => Some("high".into()),
                _ => None,
            },
            home: None,
            config_home: None,
            project: None,
            now: 100,
        };
        let mut form = setup_from(Agent::ChatGPT, &input, None, None).unwrap();
        assert_eq!(form.effort.as_deref(), Some("high"));
        assert_eq!(form.effort_model.as_deref(), Some("explicit-model"));
        assert!(!form.effort_resets());
        let entry =
            stored_from_setup(Agent::ChatGPT, &form, Some("fake-token".into()), None).unwrap();
        let persisted = serde_json::to_vec(&entry).unwrap();
        let restored: Stored = serde_json::from_slice(&persisted).unwrap();
        let resolved = connection(Agent::ChatGPT, restored, 100).unwrap();
        assert_eq!(resolved.model.as_deref(), Some("explicit-model"));
        assert_eq!(resolved.effort.as_deref(), Some("high"));
        form.model = "different-model".into();
        assert!(form.effort_resets());
        let reset =
            stored_from_setup(Agent::ChatGPT, &form, Some("fake-token".into()), None).unwrap();
        assert!(reset.effort.is_none() && reset.effort_model.is_none());
        assert!(
            connection(Agent::ChatGPT, reset, 100)
                .unwrap()
                .effort
                .is_none()
        );
        form.model = "explicit-model".into();
        assert!(!form.effort_resets());
    }

    #[test]
    fn stored_effort_with_another_model_is_refused_not_transplanted() {
        let mut entry = stored(Agent::ChatGPT, "fake-token");
        entry.effort = Some("high".into());
        entry.effort_model = Some("different-model".into());
        assert!(connection(Agent::ChatGPT, entry, 100).is_err());
    }

    #[cfg(unix)]
    struct Fixture(PathBuf);
    #[cfg(unix)]
    impl Fixture {
        fn new() -> Self {
            use std::os::unix::fs::PermissionsExt;
            let mut random = [0; 16];
            getrandom::getrandom(&mut random).unwrap();
            let path = env::temp_dir().join(format!(
                "eevee-own-source-{:x}",
                u128::from_le_bytes(random)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let path = path.canonicalize().unwrap();
            let root = super::super::posix::open_directory(&path).unwrap();
            super::super::posix::clear_new_acl(&root).unwrap();
            Self(path)
        }
        fn provider_file(&self, name: &str, contents: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let directory = self.0.join(".codex");
            if !directory.exists() {
                fs::create_dir(&directory).unwrap();
            }
            let file = directory.join(name);
            fs::write(&file, contents).unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
            file
        }
    }
    #[cfg(unix)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn provider_file_effort_survives_own_connection_save_without_provider_file_edits() {
        let fixture = Fixture::new();
        let text = "model='file-model'\nmodel_reasoning_effort='low'\n";
        let config = fixture.provider_file("config.toml", text);
        let provider_home = config.parent().unwrap().to_string_lossy().into_owned();
        let input = Inputs {
            get: |name: &str| (name == "CODEX_HOME").then(|| provider_home.clone()),
            home: None,
            config_home: None,
            project: None,
            now: 100,
        };
        let form = setup_from(Agent::ChatGPT, &input, None, None).unwrap();
        assert_eq!(form.effort_model.as_deref(), Some("file-model"));
        let entry =
            stored_from_setup(Agent::ChatGPT, &form, Some("fake-token".into()), None).unwrap();
        assert_eq!(
            connection(Agent::ChatGPT, entry, 100)
                .unwrap()
                .effort
                .as_deref(),
            Some("low")
        );
        assert_eq!(fs::read_to_string(config).unwrap(), text);
    }

    #[cfg(unix)]
    #[test]
    fn forget_removes_only_selected_own_file_and_session_and_preserves_provider_sources() {
        let fixture = Fixture::new();
        let provider_text = "{\"tokens\":{\"access_token\":\"isolated-provider-fixture\"}}";
        let provider = fixture.provider_file("auth.json", provider_text);
        let directory = storage::Directory::open(&fixture.0, None, true).unwrap();
        directory
            .write(name(Agent::OpenAI), b"own-openai-file")
            .unwrap();
        directory
            .write(name(Agent::ChatGPT), b"other-own-route")
            .unwrap();
        let mut session = HashMap::from([
            (Agent::OpenAI, stored(Agent::OpenAI, "selected-session")),
            (Agent::ChatGPT, stored(Agent::ChatGPT, "other-session")),
        ]);
        let input = Inputs {
            get: |_: &str| None,
            home: Some(fixture.0.clone()),
            config_home: None,
            project: None,
            now: 100,
        };
        let plan = prepare_forget_from(
            Agent::OpenAI,
            &input,
            session.get(&Agent::OpenAI).map(entry_identity),
        )
        .unwrap();
        assert!(!format!("{plan:?}").contains("selected-session"));
        assert!(forget_from(plan, &mut session).unwrap().changed);
        assert!(!session.contains_key(&Agent::OpenAI));
        assert_eq!(
            session[&Agent::ChatGPT].secret.as_deref(),
            Some("other-session")
        );
        assert!(directory.identity(name(Agent::OpenAI)).unwrap().is_none());
        let mut other = directory.read(name(Agent::ChatGPT)).unwrap().unwrap();
        let mut bytes = Vec::new();
        other.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"other-own-route");
        assert_eq!(fs::read_to_string(provider).unwrap(), provider_text);
        assert!(fixture.0.join(".eevee-connections").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn forget_refuses_changed_file_or_session_and_never_follows_a_provider_link() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let provider = fixture.provider_file("auth.json", "provider-must-stay");
        let directory = storage::Directory::open(&fixture.0, None, true).unwrap();
        directory.write(name(Agent::OpenAI), b"own-first").unwrap();
        let input = Inputs {
            get: |_: &str| None,
            home: Some(fixture.0.clone()),
            config_home: None,
            project: None,
            now: 100,
        };
        let mut session = HashMap::from([(Agent::OpenAI, stored(Agent::OpenAI, "first-session"))]);
        let plan = prepare_forget_from(
            Agent::OpenAI,
            &input,
            session.get(&Agent::OpenAI).map(entry_identity),
        )
        .unwrap();
        session.get_mut(&Agent::OpenAI).unwrap().secret = Some("changed-session".into());
        assert!(forget_from(plan, &mut session).is_err());
        assert!(directory.identity(name(Agent::OpenAI)).unwrap().is_some());
        let plan = prepare_forget_from(
            Agent::OpenAI,
            &input,
            session.get(&Agent::OpenAI).map(entry_identity),
        )
        .unwrap();
        directory
            .write(name(Agent::OpenAI), b"own-replaced")
            .unwrap();
        assert!(forget_from(plan, &mut session).is_err());
        let plan = prepare_forget_from(
            Agent::OpenAI,
            &input,
            session.get(&Agent::OpenAI).map(entry_identity),
        )
        .unwrap();
        let own = fixture
            .0
            .join(".eevee-connections")
            .join(name(Agent::OpenAI));
        fs::remove_file(&own).unwrap();
        symlink(&provider, &own).unwrap();
        assert!(forget_from(plan, &mut session).is_err());
        assert_eq!(fs::read_to_string(provider).unwrap(), "provider-must-stay");
        assert_eq!(
            session[&Agent::OpenAI].secret.as_deref(),
            Some("changed-session")
        );
    }

    #[test]
    fn session_only_forget_allows_existing_environment_source_without_any_environment_mutation() {
        let input = Inputs {
            get: |name: &str| (name == "OPENAI_API_KEY").then(|| "existing-env-source".into()),
            home: None,
            config_home: None,
            project: None,
            now: 100,
        };
        let mut session = HashMap::from([
            (Agent::OpenAI, stored(Agent::OpenAI, "own-session")),
            (Agent::ChatGPT, stored(Agent::ChatGPT, "other-session")),
        ]);
        let plan = prepare_forget_from(
            Agent::OpenAI,
            &input,
            session.get(&Agent::OpenAI).map(entry_identity),
        )
        .unwrap();
        assert!(forget_from(plan, &mut session).unwrap().changed);
        assert!(!session.contains_key(&Agent::OpenAI));
        assert!(session.contains_key(&Agent::ChatGPT));
        let external = resolve_with(Agent::OpenAI, &input).unwrap();
        assert_eq!(external.secret.as_deref(), Some("existing-env-source"));
    }
}
