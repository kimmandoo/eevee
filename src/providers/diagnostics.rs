//! Content-free observations only: never retain response bodies, headers or login output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Actual non-success HTTP response; not inferred from a model catalogue.
    Http(u16),
    Transport,
    /// An actual negotiated protocol / policy capability mismatch.
    Incompatible,
    /// A bounded version check failed; this is not proof of incompatibility.
    VersionCheck,
    LoginCancelled,
    LoginExpired,
    LoginFailed,
    LoginRefused,
    SignedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Authentication,
    Eligibility,
    Endpoint,
    RateLimit,
    Request,
    Transport,
    Incompatible,
    VersionCheck,
    LoginCancelled,
    LoginExpired,
    LoginFailed,
    LoginRefused,
}

impl Failure {
    pub fn kind(self) -> Kind {
        match self {
            Self::Http(401) | Self::SignedOut => Kind::Authentication,
            // A 403 is access refusal, NOT evidence that login is missing.
            Self::Http(403) => Kind::Eligibility,
            Self::Http(404 | 405) => Kind::Endpoint,
            Self::Http(429) => Kind::RateLimit,
            Self::Http(400 | 422) => Kind::Request,
            Self::Http(_) | Self::Transport => Kind::Transport,
            Self::Incompatible => Kind::Incompatible,
            Self::VersionCheck => Kind::VersionCheck,
            Self::LoginCancelled => Kind::LoginCancelled,
            Self::LoginExpired => Kind::LoginExpired,
            Self::LoginFailed => Kind::LoginFailed,
            Self::LoginRefused => Kind::LoginRefused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_actual_unauthorized_response_means_authentication_required() {
        assert_eq!(Failure::Http(401).kind(), Kind::Authentication);
        assert_eq!(Failure::Http(403).kind(), Kind::Eligibility);
        assert_eq!(Failure::Http(404).kind(), Kind::Endpoint);
        assert_eq!(Failure::Incompatible.kind(), Kind::Incompatible);
        assert_eq!(Failure::VersionCheck.kind(), Kind::VersionCheck);
        assert_eq!(Failure::Http(500).kind(), Kind::Transport);
    }
}
