//! The single error type shared by the library and the CLI.

use std::fmt;
use std::io;

use crate::provider::UnknownProvider;
use crate::validate::ValidationError;

/// Exit status for success (including a search with zero results).
pub const EXIT_OK: u8 = 0;
/// Exit status when the request or response failed.
pub const EXIT_FAILURE: u8 = 1;
/// Exit status for usage, configuration or input errors (nothing was sent).
pub const EXIT_USAGE: u8 = 2;

/// A failure reported by the Cloudflare API.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApiError {
    /// HTTP status of the response.
    pub status: u16,
    /// Machine-readable code, e.g. `web_search_payment_required` or `7000`.
    pub code: Option<String>,
    /// Human-readable message from Cloudflare, sanitised.
    pub message: Option<String>,
    /// Field-level details, e.g. `limit: Number must be less than or equal to 10`.
    pub details: Vec<String>,
    /// AI Gateway request ID, for support and log lookup.
    pub request_id: Option<String>,
}

impl ApiError {
    /// A suggestion for fixing the failure, when one is known.
    #[must_use]
    pub fn hint(&self) -> Option<&'static str> {
        let code = self.code.as_deref().unwrap_or_default();
        match (self.status, code) {
            (402, _) | (_, "web_search_payment_required") => Some(
                "the gateway has no funding for this provider: load AI Gateway credits \
                 (AI Gateway > Manage > Top-up credits) or store a provider key and pass --byok-alias",
            ),
            (401 | 403, _) | (_, "9106" | "10000") => Some(
                "check CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID; the token needs \
                 Account > Workers AI > Read and Account > AI Gateway > Read",
            ),
            (_, "2001") => Some(
                "that AI Gateway does not exist in this account: create it in the dashboard \
                 or choose another with --gateway / WS_GATEWAY_ID",
            ),
            (_, "web_search_byok_not_configured") => Some(
                "no provider key with that alias is stored on the gateway \
                 (AI Gateway > Provider Keys); Cloudflare does not fall back to credits",
            ),
            (429, _) => {
                Some("rate limited; wait before retrying (ws never retries paid requests itself)")
            }
            (400, _) => Some("check the gateway ID, provider and BYOK alias"),
            (500..=599, _) => Some("Cloudflare or the provider failed; retrying later may help"),
            _ => None,
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cloudflare API error: HTTP {}", self.status)?;
        if let Some(code) = &self.code {
            write!(f, " [{code}]")?;
        }
        if let Some(message) = &self.message {
            write!(f, " {message}")?;
        }
        for detail in &self.details {
            write!(f, "\n  - {detail}")?;
        }
        if let Some(id) = &self.request_id {
            write!(f, "\n  request id: {id}")?;
        }
        if let Some(hint) = self.hint() {
            write!(f, "\n  hint: {hint}")?;
        }
        Ok(())
    }
}

/// Everything that can go wrong in `ws`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Missing or malformed configuration (environment, flags, config file).
    #[error("configuration error: {0}")]
    Config(String),
    /// Input rejected by local validation.
    #[error("invalid input: {0}")]
    Invalid(#[from] ValidationError),
    /// An unrecognised provider name.
    #[error("invalid input: {0}")]
    Provider(#[from] UnknownProvider),
    /// The request never produced an HTTP response (DNS, TLS, timeout, ...).
    #[error("request failed: {0}")]
    Transport(String),
    /// Cloudflare answered with an error.
    #[error("{0}")]
    Api(ApiError),
    /// Cloudflare answered with something that is not a search result.
    #[error("unexpected response: {0}")]
    UnexpectedResponse(String),
    /// Reading or writing a local file or stream failed.
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
}

impl Error {
    /// The process exit status this error maps to.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_) | Self::Invalid(_) | Self::Provider(_) => EXIT_USAGE,
            Self::Transport(_) | Self::Api(_) | Self::UnexpectedResponse(_) | Self::Io(_) => {
                EXIT_FAILURE
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_separate_local_from_remote_failures() {
        assert_eq!(Error::Config("x".into()).exit_code(), EXIT_USAGE);
        assert_eq!(
            Error::Invalid(ValidationError::EmptyQuery).exit_code(),
            EXIT_USAGE
        );
        assert_eq!(
            Error::Provider(UnknownProvider("x".into())).exit_code(),
            EXIT_USAGE
        );
        assert_eq!(Error::Transport("x".into()).exit_code(), EXIT_FAILURE);
        assert_eq!(Error::Api(ApiError::default()).exit_code(), EXIT_FAILURE);
        assert_eq!(
            Error::UnexpectedResponse("x".into()).exit_code(),
            EXIT_FAILURE
        );
    }

    #[test]
    fn api_error_display_includes_code_details_request_and_hint() {
        let err = ApiError {
            status: 402,
            code: Some("web_search_payment_required".into()),
            message: None,
            details: vec!["limit: too big".into()],
            request_id: Some("req-1".into()),
        };
        let text = err.to_string();
        assert!(text.starts_with("Cloudflare API error: HTTP 402 [web_search_payment_required]"));
        assert!(text.contains("- limit: too big"));
        assert!(text.contains("request id: req-1"));
        assert!(text.contains("hint: the gateway has no funding"));
    }

    #[test]
    fn hints_by_status() {
        let with = |status, code: Option<&str>| ApiError {
            status,
            code: code.map(str::to_owned),
            ..ApiError::default()
        };
        assert!(
            with(401, None)
                .hint()
                .unwrap()
                .contains("CLOUDFLARE_API_TOKEN")
        );
        assert!(
            with(400, Some("9106"))
                .hint()
                .unwrap()
                .contains("CLOUDFLARE_API_TOKEN")
        );
        assert!(
            with(400, Some("7000"))
                .hint()
                .unwrap()
                .contains("gateway ID")
        );
        assert!(with(429, None).hint().unwrap().contains("rate limited"));
        assert!(with(503, None).hint().unwrap().contains("retrying later"));
        assert_eq!(with(418, None).hint(), None);
    }
}
