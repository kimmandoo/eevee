//! Typed public classification of provider failures. Only short machine
//! tokens are read (error type/code/status, finish/stop/incomplete reasons,
//! numeric status); provider prose, parameters and bodies are never retained
//! or shown. Classification adds a cause and an explicit next step; it never
//! changes the outcome, retries or resends.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Overloaded,
    RateLimit,
    Quota,
    ContextLimit,
    OutputLimit,
    ContentPolicy,
    InvalidRequest,
    Authentication,
    Permission,
    NotFound,
    Server,
    Timeout,
    Unclassified,
}

impl Class {
    fn label(self) -> &'static str {
        match self {
            Self::Overloaded => "provider is temporarily overloaded or unavailable",
            Self::RateLimit => "provider rate limit reached",
            Self::Quota => "account usage quota or billing limit reached",
            Self::ContextLimit => "request exceeds the model's context or size limit",
            Self::OutputLimit => "model output limit reached",
            Self::ContentPolicy => "blocked by the provider's content policy",
            Self::InvalidRequest => "provider rejected the request shape or settings",
            Self::Authentication => "credentials were rejected or expired",
            Self::Permission => "access denied for this account, model or region",
            Self::NotFound => "model or endpoint is unavailable",
            Self::Server => "provider internal error",
            Self::Timeout => "provider-side timeout",
            Self::Unclassified => "unclassified provider error",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Overloaded | Self::Server => {
                "wait briefly, then retry explicitly or choose another model"
            }
            Self::RateLimit => "wait for the provider's limit window, then retry explicitly",
            Self::Quota => {
                "check plan, credits or billing for the selected account; an unchanged retry will fail again"
            }
            Self::ContextLimit => {
                "reduce attached sources/context or start a fresh chat (/chat-new), then retry"
            }
            Self::OutputLimit => "ask for a smaller step, then continue with a follow-up",
            Self::ContentPolicy => {
                "rephrase the request; an unchanged retry is likely to be blocked again"
            }
            Self::InvalidRequest => {
                "check the model, effort and request settings for this provider"
            }
            Self::Authentication => "update the selected connection's authentication",
            Self::Permission => "check provider permissions for the selected model",
            Self::NotFound => "check the base URL and model selection",
            Self::Timeout => "retry explicitly, or ask for a smaller step",
            Self::Unclassified => {
                "inspect completed effects; retry explicitly or change the request"
            }
        }
    }
}

/// A classified failure with the provider's own short code when it is a safe
/// identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Fault {
    pub class: Class,
    pub code: Option<String>,
}

impl Fault {
    /// `lead` names the failure kind; the result never contains provider prose.
    pub(super) fn describe(&self, lead: &str) -> String {
        let code = self
            .code
            .as_deref()
            .map_or_else(String::new, |code| format!(" (code {code})"));
        format!(
            "{lead}: {}{code}; {}; private error body omitted",
            self.class.label(),
            self.class.action()
        )
    }

    /// Appended to a status-only message, e.g. an HTTP rejection.
    pub(super) fn detail(&self) -> String {
        let code = self
            .code
            .as_deref()
            .map_or_else(String::new, |code| format!("code {code}: "));
        format!(
            "provider reported {code}{}; {}",
            self.class.label(),
            self.class.action()
        )
    }
}

/// Short identifiers only; anything resembling prose is not displayed.
fn safe(token: &str) -> Option<&str> {
    (!token.is_empty()
        && token.len() <= 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
    .then_some(token)
}

/// Maps one provider token (any protocol, any case) to a public class.
pub(super) fn classify(token: &str) -> Class {
    let token = token.to_ascii_lowercase();
    match token.as_str() {
        "overloaded_error"
        | "overloaded"
        | "server_is_overloaded"
        | "engine_overloaded"
        | "unavailable"
        | "service_unavailable"
        | "slow_down"
        | "capacity_exceeded" => Class::Overloaded,
        "rate_limit_error"
        | "rate_limit_exceeded"
        | "rate_limited"
        | "too_many_requests"
        | "resource_exhausted"
        | "requests"
        | "tokens" => Class::RateLimit,
        "insufficient_quota"
        | "quota_exceeded"
        | "usage_limit_reached"
        | "usage_not_included"
        | "billing_error"
        | "billing_hard_limit_reached"
        | "billing_not_active"
        | "credit_balance_too_low"
        | "payment_required" => Class::Quota,
        "context_length_exceeded"
        | "context_window_exceeded"
        | "model_context_window_exceeded"
        | "request_too_large"
        | "string_above_max_length"
        | "prompt_too_long"
        | "input_too_long"
        | "token_limit_exceeded"
        | "payload_too_large" => Class::ContextLimit,
        "max_tokens" | "max_output_tokens" | "length" | "max_tokens_exceeded" => Class::OutputLimit,
        "content_filter"
        | "content_policy_violation"
        | "refusal"
        | "safety"
        | "recitation"
        | "blocklist"
        | "prohibited_content"
        | "spii"
        | "image_safety"
        | "invalid_prompt"
        | "cyber_policy" => Class::ContentPolicy,
        "invalid_request_error"
        | "invalid_request"
        | "invalid_argument"
        | "failed_precondition"
        | "bad_request"
        | "unsupported_parameter"
        | "unsupported_value"
        | "invalid_value"
        | "malformed_function_call"
        | "unprocessable_entity" => Class::InvalidRequest,
        "authentication_error"
        | "invalid_api_key"
        | "unauthenticated"
        | "unauthorized"
        | "invalid_authentication"
        | "token_expired"
        | "expired_token" => Class::Authentication,
        "permission_error"
        | "permission_denied"
        | "forbidden"
        | "access_denied"
        | "unsupported_country_region_territory"
        | "model_not_allowed" => Class::Permission,
        "not_found_error" | "not_found" | "model_not_found" => Class::NotFound,
        "api_error"
        | "server_error"
        | "internal_error"
        | "internal"
        | "internal_server_error"
        | "bad_gateway" => Class::Server,
        "deadline_exceeded" | "timeout" | "request_timeout" | "gateway_timeout" => Class::Timeout,
        _ => token.parse::<u16>().map_or(Class::Unclassified, status),
    }
}

/// HTTP-style numeric status codes, including those embedded in envelopes.
pub(super) fn status(code: u16) -> Class {
    match code {
        400 | 422 => Class::InvalidRequest,
        401 => Class::Authentication,
        402 => Class::Quota,
        403 => Class::Permission,
        404 => Class::NotFound,
        408 | 504 => Class::Timeout,
        413 => Class::ContextLimit,
        429 => Class::RateLimit,
        503 | 529 => Class::Overloaded,
        500..=599 => Class::Server,
        _ => Class::Unclassified,
    }
}

fn token(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => safe(text.trim()).map(str::to_owned),
        Value::Number(number) => number.as_u64().map(|number| number.to_string()),
        _ => None,
    }
}

/// Classifies tokens in priority order: the first one that maps to a known
/// class wins, so a specific `code` outranks a generic `type`.
pub(super) fn from_tokens<'a>(tokens: impl IntoIterator<Item = Option<&'a Value>>) -> Fault {
    let tokens = tokens.into_iter().filter_map(token).collect::<Vec<_>>();
    tokens
        .iter()
        .find_map(|token| {
            let class = classify(token);
            (class != Class::Unclassified).then(|| Fault {
                class,
                code: Some(token.clone()),
            })
        })
        .unwrap_or_else(|| Fault {
            class: Class::Unclassified,
            code: tokens
                .into_iter()
                .find(|token| token.parse::<u16>().is_err()),
        })
}

/// Error envelopes across Responses/Codex, Chat Completions, Messages and
/// Gemini: `{error:{code,type,status}}`, `{type:"error",code}` and
/// `{response:{error:{code}}}`.
pub(super) fn from_envelope(value: &Value) -> Fault {
    let error = value
        .get("error")
        .filter(|error| error.is_object())
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("error"))
        })
        .filter(|error| error.is_object());
    let field = |name: &str| error.and_then(|error| error.get(name));
    from_tokens([
        field("code"),
        field("status"),
        field("type"),
        value.get("code"),
        value.get("status").filter(|_| error.is_none()),
        // A bare `{error:"..."}` string may itself be a code.
        value.get("error").filter(|error| error.is_string()),
    ])
}

/// A bounded non-success HTTP body; `None` when it carries no typed token.
pub(super) fn from_body(bytes: &[u8]) -> Option<Fault> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    // Some providers wrap the envelope in a one-element array.
    let value = match &value {
        Value::Array(items) => items.first()?,
        value => value,
    };
    let fault = from_envelope(value);
    (fault.class != Class::Unclassified || fault.code.is_some()).then_some(fault)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_protocol_envelope_maps_to_a_public_class_without_prose() {
        for (envelope, class, code) in [
            (
                json!({"type":"error","error":{"type":"overloaded_error","message":"private prose"}}),
                Class::Overloaded,
                "overloaded_error",
            ),
            (
                json!({"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"private prose"}}),
                Class::ContextLimit,
                "context_length_exceeded",
            ),
            (
                json!({"type":"error","code":"rate_limit_exceeded","message":"private prose"}),
                Class::RateLimit,
                "rate_limit_exceeded",
            ),
            (
                json!({"type":"response.failed","response":{"error":{"code":"usage_limit_reached","message":"private prose"}}}),
                Class::Quota,
                "usage_limit_reached",
            ),
            (
                json!({"error":{"code":503,"status":"UNAVAILABLE","message":"private prose"}}),
                Class::Overloaded,
                "503",
            ),
            (
                json!({"error":{"type":"api_error","message":"private prose"}}),
                Class::Server,
                "api_error",
            ),
        ] {
            let fault = from_envelope(&envelope);
            assert_eq!((fault.class, fault.code.as_deref()), (class, Some(code)));
            let message = fault.describe("Provider reported an inference error");
            assert!(message.starts_with("Provider reported an inference error: "));
            assert!(message.contains(class.action()), "{message}");
            assert!(!message.contains("private prose"), "{message}");
        }
    }

    #[test]
    fn unknown_or_unsafe_tokens_stay_unclassified_and_never_echo_prose() {
        let fault = from_envelope(&json!({"error":{"code":"brand_new_code"}}));
        assert_eq!(
            (fault.class, fault.code.as_deref()),
            (Class::Unclassified, Some("brand_new_code"))
        );
        let fault = from_envelope(&json!({"error":{"code":"Bearer sk-secret leaked prose"}}));
        assert_eq!((fault.class, fault.code), (Class::Unclassified, None));
        let fault = from_envelope(&json!({"error":{"message":"private prose"}}));
        assert_eq!(
            (fault.class, fault.code.as_deref()),
            (Class::Unclassified, None)
        );
        assert!(!fault.describe("lead").contains("prose"));
        assert_eq!(
            from_tokens([Some(&json!("x".repeat(65)))]).code,
            None,
            "overlong tokens are not identifiers"
        );
    }

    #[test]
    fn finish_reasons_and_bodies_classify_across_protocols() {
        for (reason, class) in [
            ("length", Class::OutputLimit),
            ("max_tokens", Class::OutputLimit),
            ("MAX_TOKENS", Class::OutputLimit),
            ("max_output_tokens", Class::OutputLimit),
            ("content_filter", Class::ContentPolicy),
            ("SAFETY", Class::ContentPolicy),
            ("refusal", Class::ContentPolicy),
            ("model_context_window_exceeded", Class::ContextLimit),
        ] {
            assert_eq!(classify(reason), class, "{reason}");
        }
        let body = br#"{"error":{"message":"private","type":"invalid_request_error","code":"context_length_exceeded"}}"#;
        assert_eq!(from_body(body).unwrap().class, Class::ContextLimit);
        let body = br#"[{"error":{"code":429,"status":"RESOURCE_EXHAUSTED"}}]"#;
        assert_eq!(from_body(body).unwrap().class, Class::RateLimit);
        assert_eq!(from_body(b"private plain text"), None);
        assert_eq!(from_body(br#"{"error":{"message":"private"}}"#), None);
        assert!(
            from_body(br#"{"error":{"type":"insufficient_quota"}}"#)
                .unwrap()
                .detail()
                .contains("billing")
        );
    }
}
