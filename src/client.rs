//! The Cloudflare Web Search REST client and its wire types.
//!
//! `POST {base}/accounts/{account_id}/ai/websearch/` with a bearer token.
//! Searches are billed and no idempotency key exists, so nothing here retries.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::body::read_text;
use crate::config::Credentials;
use crate::error::{ApiError, Error};
use crate::output::sanitize;
use crate::provider::Provider;

/// Production API root.
pub const DEFAULT_BASE_URL: &str = "https://api.cloudflare.com/client/v4";
/// Largest response body accepted (after decompression), as local protection
/// against runaway responses.
pub const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;
/// Longest excerpt of an unparseable error body shown to the user.
const EXCERPT_CHARS: usize = 200;
/// Header carrying the AI Gateway request ID.
pub(crate) const REQUEST_ID_HEADER: &str = "cf-aig-request-id";

/// The JSON body of a search request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchRequest<'a> {
    /// The search query (1-1,024 characters).
    pub query: &'a str,
    /// The provider to search with.
    pub provider: Provider,
    /// Maximum number of results (1-10).
    pub limit: u8,
    /// Stored provider key to bill; omitted from the body when `None`.
    #[serde(rename = "byokAlias", skip_serializing_if = "Option::is_none")]
    pub byok_alias: Option<&'a str>,
    /// Gateway selection.
    pub options: RequestOptions<'a>,
}

/// The `options` object of a search request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequestOptions<'a> {
    /// The AI Gateway to route through.
    pub gateway: GatewayRef<'a>,
}

/// The `options.gateway` object of a search request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GatewayRef<'a> {
    /// Gateway ID.
    pub id: &'a str,
}

impl<'a> SearchRequest<'a> {
    /// Builds a request for already-validated values.
    #[must_use]
    pub const fn new(
        query: &'a str,
        provider: Provider,
        limit: u8,
        byok_alias: Option<&'a str>,
        gateway_id: &'a str,
    ) -> Self {
        Self {
            query,
            provider,
            limit,
            byok_alias,
            options: RequestOptions {
                gateway: GatewayRef { id: gateway_id },
            },
        }
    }
}

/// One search result. Fields Cloudflare adds later are preserved in `extra`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Source URL.
    pub url: String,
    /// Page title.
    pub title: String,
    /// Snippet or excerpt, when the provider returns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Representative image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Site favicon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon_url: Option<String>,
    /// Last-modified timestamp (not necessarily the publication date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified_date: Option<String>,
    /// Undocumented fields, passed through untouched.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Response metadata. Every field is optional upstream.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    /// The query as Cloudflare saw it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Request ID for log lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Upstream latency in milliseconds (a JSON number, possibly fractional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    /// Undocumented fields, passed through untouched.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A successful search response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    /// Results; may legitimately be empty.
    pub items: Vec<Item>,
    /// Response metadata, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// A search response plus what the client observed around it.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOutcome {
    /// The decoded response.
    pub response: SearchResponse,
    /// Request ID from metadata, falling back to the `cf-aig-request-id` header.
    pub request_id: Option<String>,
    /// Wall-clock duration measured locally.
    pub elapsed: Duration,
}

pub(crate) fn excerpt(body: &str) -> String {
    let clean = sanitize(body);
    let trimmed = clean.trim();
    let mut out: String = trimmed.chars().take(EXCERPT_CHARS).collect();
    if trimmed.chars().count() > EXCERPT_CHARS {
        out.push('…');
    }
    out
}

fn text_field(object: &Value, key: &str) -> Option<String> {
    match object.get(key) {
        Some(Value::String(s)) => Some(sanitize(s)),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn detail(message: &Value) -> Option<String> {
    let text = text_field(message, "message")?;
    let path = message.get("path").and_then(Value::as_array).map(|parts| {
        parts
            .iter()
            .map(|p| p.as_str().map_or_else(|| p.to_string(), sanitize))
            .collect::<Vec<_>>()
            .join(".")
    });
    Some(match path {
        Some(path) if !path.is_empty() => format!("{path}: {text}"),
        _ => text,
    })
}

/// Decodes an error response.
///
/// Three envelopes have been observed live:
/// `{"ok":false,"error":{"code":..,"gatewayRequestId":..}}` from web search,
/// `{"success":false,"errors":[{"code":..,"message":..}],"messages":[..]}` from
/// the Cloudflare API, and `{"success":false,"error":[{"code":..,"message":..}]}`
/// from AI Gateway. Anything else is reported as a bounded, sanitised excerpt.
#[must_use]
pub fn parse_api_error(status: u16, body: &str, header_request_id: Option<String>) -> ApiError {
    let mut error = ApiError {
        status,
        request_id: header_request_id,
        ..ApiError::default()
    };
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        let text = excerpt(body);
        error.message = (!text.is_empty()).then(|| format!("(non-JSON body) {text}"));
        return error;
    };

    let listed = |key: &str| {
        json.get(key)
            .and_then(Value::as_array)
            .and_then(|e| e.first())
    };
    if let Some(gateway) = json.get("error").filter(|e| e.is_object()) {
        error.code = text_field(gateway, "code");
        error.message = text_field(gateway, "message");
        if let Some(id) = text_field(gateway, "gatewayRequestId") {
            error.request_id = Some(id);
        }
    } else if let Some(first) = listed("errors").or_else(|| listed("error")) {
        error.code = text_field(first, "code");
        error.message = text_field(first, "message");
        error.details = json
            .get("messages")
            .and_then(Value::as_array)
            .map(|messages| messages.iter().filter_map(detail).collect())
            .unwrap_or_default();
    } else {
        error.message = Some(excerpt(body));
    }
    error
}

fn declares_failure(json: &Value) -> bool {
    ["ok", "success"]
        .iter()
        .any(|key| json.get(*key) == Some(&Value::Bool(false)))
}

/// Decodes a 2xx body.
///
/// The documented shape is a bare `{items, metadata}`
/// object; the same object nested under `result` is also accepted because the
/// neighbouring Cloudflare APIs use that envelope. A body without `items` is an
/// error rather than a fabricated empty result, and a 2xx body that declares
/// failure is never treated as success.
///
/// # Errors
/// [`Error::Api`] if the body declares failure, [`Error::UnexpectedResponse`]
/// if it is not a search result.
pub fn parse_success(
    status: u16,
    body: &str,
    header_request_id: Option<String>,
) -> Result<SearchResponse, Error> {
    let json: Value = serde_json::from_str(body)
        .map_err(|_| Error::UnexpectedResponse(format!("body is not JSON: {}", excerpt(body))))?;
    if declares_failure(&json) {
        return Err(Error::Api(parse_api_error(status, body, header_request_id)));
    }
    let payload = if json.get("items").is_some() {
        json
    } else {
        match json.get("result") {
            Some(result) if result.get("items").is_some() => result.clone(),
            _ => {
                return Err(Error::UnexpectedResponse(format!(
                    "no 'items' in response: {}",
                    excerpt(body)
                )));
            }
        }
    };
    serde_json::from_value(payload)
        .map_err(|e| Error::UnexpectedResponse(format!("malformed search result: {e}")))
}

/// A reusable HTTP client for the Web Search endpoint.
#[derive(Debug, Clone)]
pub struct Client {
    agent: ureq::Agent,
    base_url: String,
}

impl Client {
    /// Creates a client. Redirects are never followed, so the bearer token
    /// cannot be forwarded to another host.
    #[must_use]
    pub fn new(base_url: &str, timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(concat!("ws/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            agent,
            base_url: base_url.trim_end_matches('/').to_owned(),
        }
    }

    /// The endpoint URL for an account (note the trailing slash).
    #[must_use]
    pub fn endpoint(&self, account_id: &str) -> String {
        format!("{}/accounts/{account_id}/ai/websearch/", self.base_url)
    }

    /// Runs one search. Makes exactly one HTTP request.
    ///
    /// # Errors
    /// [`Error::Transport`] when no response arrives, [`Error::Api`] for
    /// non-2xx responses, [`Error::UnexpectedResponse`] for undecodable ones.
    pub fn search(
        &self,
        credentials: &Credentials,
        request: &SearchRequest<'_>,
    ) -> Result<SearchOutcome, Error> {
        let body = serde_json::to_string(request)
            .map_err(|e| Error::UnexpectedResponse(format!("cannot encode request: {e}")))?;
        let started = Instant::now();
        let mut response = self
            .agent
            .post(self.endpoint(&credentials.account_id))
            .header("Authorization", format!("Bearer {}", credentials.token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .send(body)
            .map_err(|e| transport_error(&e))?;

        let status = response.status().as_u16();
        let header_request_id = response
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(sanitize);
        let text = read_text(&mut response, MAX_BODY_BYTES).map_err(|e| transport_error(&e))?;
        let elapsed = started.elapsed();

        if !(200..300).contains(&status) {
            return Err(Error::Api(parse_api_error(
                status,
                &text,
                header_request_id,
            )));
        }
        let response = parse_success(status, &text, header_request_id.clone())?;
        let request_id = response
            .metadata
            .as_ref()
            .and_then(|m| m.request_id.as_deref())
            .map(sanitize)
            .or(header_request_id);
        Ok(SearchOutcome {
            response,
            request_id,
            elapsed,
        })
    }
}

pub(crate) fn transport_error(error: &ureq::Error) -> Error {
    Error::Transport(if matches!(error, ureq::Error::Timeout(_)) {
        "timed out waiting for Cloudflare (see --timeout)".to_owned()
    } else if let ureq::Error::BodyExceedsLimit(limit) = error {
        format!("response body exceeds the {limit}-byte safety limit")
    } else {
        sanitize(&error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn request_matches_documented_shape() {
        let request = SearchRequest::new("rust", Provider::Exa, 5, Some("my-key"), "gw");
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "query": "rust",
                "provider": "exa",
                "limit": 5,
                "byokAlias": "my-key",
                "options": { "gateway": { "id": "gw" } }
            })
        );
    }

    #[test]
    fn absent_alias_is_omitted_not_null() {
        let request = SearchRequest::new("rust", Provider::Ceramic, 10, None, "default");
        let value = serde_json::to_value(&request).unwrap();
        assert!(value.get("byokAlias").is_none());
        assert_eq!(value["options"]["gateway"]["id"], "default");
    }

    #[test]
    fn endpoint_has_trailing_slash_and_tolerates_base_slash() {
        let client = Client::new(
            "https://api.cloudflare.com/client/v4/",
            Duration::from_secs(1),
        );
        assert_eq!(
            client.endpoint("abc"),
            "https://api.cloudflare.com/client/v4/accounts/abc/ai/websearch/"
        );
    }

    #[test]
    fn parses_documented_response_with_optional_and_unknown_fields() {
        let body = json!({
            "items": [
                { "url": "https://a.example", "title": "A", "description": "d",
                  "imageUrl": "https://a.example/i.png", "faviconUrl": "https://a.example/f.ico",
                  "lastModifiedDate": "2026-10-02T12:00:00Z", "score": 0.9 },
                { "url": "https://b.example", "title": "B" }
            ],
            "metadata": { "query": "q", "requestId": "r-1", "latencyMs": 612.5, "cached": false }
        })
        .to_string();
        let response = parse_success(200, &body, None).unwrap();
        assert_eq!(response.items.len(), 2);
        assert_eq!(
            response.items[0].image_url.as_deref(),
            Some("https://a.example/i.png")
        );
        assert_eq!(response.items[0].extra["score"], json!(0.9));
        assert_eq!(response.items[1].description, None);
        let metadata = response.metadata.clone().unwrap();
        assert_eq!(metadata.request_id.as_deref(), Some("r-1"));
        assert_eq!(metadata.latency_ms, Some(612.5));
        assert_eq!(metadata.extra["cached"], json!(false));
        // Round trip keeps unknown fields and camelCase names.
        let again = serde_json::to_value(&response).unwrap();
        assert_eq!(
            again["items"][0]["lastModifiedDate"],
            "2026-10-02T12:00:00Z"
        );
        assert_eq!(again["items"][0]["score"], json!(0.9));
        assert!(again["items"][1].get("description").is_none());
    }

    #[test]
    fn empty_items_is_a_successful_empty_result() {
        let response = parse_success(200, r#"{"items":[]}"#, None).unwrap();
        assert!(response.items.is_empty());
        assert_eq!(response.metadata, None);
    }

    #[test]
    fn result_envelope_is_unwrapped() {
        let body = r#"{"success":true,"result":{"items":[{"url":"u","title":"t"}]}}"#;
        assert_eq!(parse_success(200, body, None).unwrap().items.len(), 1);
    }

    #[test]
    fn missing_items_is_an_error() {
        for body in [
            "{}",
            r#"{"metadata":{}}"#,
            r#"{"result":null}"#,
            "[]",
            "null",
        ] {
            let err = parse_success(200, body, None).unwrap_err();
            assert!(matches!(err, Error::UnexpectedResponse(_)), "{body}: {err}");
        }
    }

    #[test]
    fn malformed_bodies_are_errors() {
        for body in [
            "",
            "<html>oops</html>",
            r#"{"items":[{"title":"no url"}]}"#,
            r#"{"items":7}"#,
        ] {
            let err = parse_success(200, body, None).unwrap_err();
            assert!(matches!(err, Error::UnexpectedResponse(_)), "{body}: {err}");
        }
    }

    #[test]
    fn success_status_with_failure_body_is_an_api_error() {
        let body = r#"{"ok":false,"error":{"code":"boom","status":500}}"#;
        let err = parse_success(200, body, Some("hdr".into())).unwrap_err();
        assert!(
            matches!(&err, Error::Api(e) if e.code.as_deref() == Some("boom")),
            "{err}"
        );
    }

    #[test]
    fn parses_observed_gateway_error_envelope() {
        // Captured from the live API on 2026-10-02 (request ID replaced).
        let body = r#"{"ok":false,"error":{"category":"gateway","code":"web_search_payment_required","status":402,"retryable":false,"gatewayRequestId":"req-body"}}"#;
        let error = parse_api_error(402, body, Some("req-header".into()));
        assert_eq!(error.status, 402);
        assert_eq!(error.code.as_deref(), Some("web_search_payment_required"));
        assert_eq!(error.request_id.as_deref(), Some("req-body"));
        assert_eq!(error.message, None);
    }

    #[test]
    fn parses_observed_api_error_envelope_with_field_details() {
        // Captured from the live API on 2026-10-02.
        let body = r#"{"success":false,"errors":[{"code":7000,"message":"Invalid web search request body"}],"messages":[{"code":"too_big","maximum":10,"type":"number","inclusive":true,"exact":false,"message":"Number must be less than or equal to 10","path":["limit"]}],"result":null}"#;
        let error = parse_api_error(400, body, Some("req-header".into()));
        assert_eq!(error.code.as_deref(), Some("7000"));
        assert_eq!(
            error.message.as_deref(),
            Some("Invalid web search request body")
        );
        assert_eq!(
            error.details,
            vec!["limit: Number must be less than or equal to 10"]
        );
        assert_eq!(error.request_id.as_deref(), Some("req-header"));
    }

    #[test]
    fn parses_observed_gateway_error_list_envelope() {
        // Captured from the live API on 2026-10-02 for a gateway that does not exist.
        let body = r#"{"success":false,"result":[],"messages":[],"error":[{"code":2001,"message":"Please configure AI Gateway in the Cloudflare dashboard"}],"name":"AiGatewayError","httpCode":400,"internalCode":2001}"#;
        let error = parse_api_error(400, body, Some("req-header".into()));
        assert_eq!(error.code.as_deref(), Some("2001"));
        assert_eq!(
            error.message.as_deref(),
            Some("Please configure AI Gateway in the Cloudflare dashboard")
        );
        assert!(error.hint().unwrap().contains("--gateway"));
    }

    #[test]
    fn parses_observed_missing_byok_alias_error() {
        // Captured from the live API on 2026-10-02 for an alias that is not stored.
        let body = r#"{"ok":false,"error":{"category":"gateway","code":"web_search_byok_not_configured","status":400,"retryable":false,"gatewayRequestId":"r"}}"#;
        let error = parse_api_error(400, body, None);
        assert_eq!(
            error.code.as_deref(),
            Some("web_search_byok_not_configured")
        );
        assert!(error.hint().unwrap().contains("Provider Keys"));
    }

    #[test]
    fn non_json_error_bodies_are_bounded_and_sanitised() {
        let html = format!("<html>\u{1b}[31m{}</html>", "x".repeat(500));
        let error = parse_api_error(502, &html, None);
        let message = error.message.unwrap();
        assert!(message.starts_with("(non-JSON body) <html>"));
        assert!(!message.contains('\u{1b}'));
        assert!(message.chars().count() < 230);
        assert!(message.ends_with('…'));

        assert_eq!(parse_api_error(500, "", None).message, None);
        let unknown = parse_api_error(500, r#"{"weird":true}"#, None);
        assert_eq!(unknown.message.as_deref(), Some(r#"{"weird":true}"#));
    }
}
