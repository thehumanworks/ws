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
        self.describe(f, "Cloudflare API error", self.hint())
    }
}

/// An [`ApiError`] from Browser Run (`ws fetch --render`), which needs its own
/// hints: the permissions and limits differ from web search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub ApiError);

impl RenderError {
    /// A suggestion for fixing the failure, when one is known.
    #[must_use]
    pub const fn hint(&self) -> Option<&'static str> {
        match self.0.status {
            401 | 403 => Some(
                "--render needs CLOUDFLARE_API_TOKEN with Account > Browser Rendering > Edit, \
                 and CLOUDFLARE_ACCOUNT_ID",
            ),
            429 => Some("Browser Run rate limit reached; wait before retrying (ws never retries)"),
            422 => Some("Browser Run could not load the page; check the URL, or raise --timeout"),
            500..=599 => Some("Cloudflare or the page failed; retrying later may help"),
            _ => None,
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .describe(f, "Cloudflare Browser Run error", self.hint())
    }
}

impl ApiError {
    fn describe(&self, f: &mut fmt::Formatter<'_>, label: &str, hint: Option<&str>) -> fmt::Result {
        write!(f, "{label}: HTTP {}", self.status)?;
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
        if let Some(hint) = hint {
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
    /// A fetched page answered with a non-success HTTP status.
    #[error("fetch failed: HTTP {status} from {url}")]
    PageStatus {
        /// HTTP status of the final response.
        status: u16,
        /// The URL that answered (after redirects), sanitised.
        url: String,
    },
    /// The page's host is not on the public internet and `--allow-private` was not given.
    #[error("blocked: {0}")]
    PageBlocked(String),
    /// Cloudflare Browser Run answered with an error (`ws fetch --render`).
    #[error("{0}")]
    Render(RenderError),
    /// A fetched page is not something `ws` can show as text.
    #[error("cannot read page: {0}")]
    PageContent(String),
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
            Self::Transport(_)
            | Self::Api(_)
            | Self::UnexpectedResponse(_)
            | Self::PageStatus { .. }
            | Self::PageContent(_)
            | Self::PageBlocked(_)
            | Self::Render(_)
            | Self::Io(_) => EXIT_FAILURE,
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
        let status = Error::PageStatus {
            status: 404,
            url: "https://a.example/x".into(),
        };
        assert_eq!(status.exit_code(), EXIT_FAILURE);
        assert_eq!(
            status.to_string(),
            "fetch failed: HTTP 404 from https://a.example/x"
        );
        assert_eq!(Error::PageContent("x".into()).exit_code(), EXIT_FAILURE);
        assert_eq!(Error::PageBlocked("x".into()).exit_code(), EXIT_FAILURE);
        let render = Error::Render(RenderError(ApiError {
            status: 403,
            code: Some("10000".into()),
            message: Some("Authentication error".into()),
            ..ApiError::default()
        }));
        assert_eq!(render.exit_code(), EXIT_FAILURE);
        let text = render.to_string();
        assert!(
            text.starts_with("Cloudflare Browser Run error: HTTP 403 [10000] Authentication error"),
            "{text}"
        );
        assert!(text.contains("Browser Rendering > Edit"), "{text}");
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
