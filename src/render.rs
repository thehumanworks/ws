//! Rendering a page in Cloudflare's Browser Run for `ws fetch --render`.
//!
//! `POST {base}/accounts/{account_id}/browser-run/content` with a bearer token
//! returns the page's HTML after JavaScript has run. Unlike [`crate::fetch`],
//! this is a Cloudflare API call: it carries credentials, uses browser time
//! that Cloudflare meters, never follows redirects and is never retried. The
//! result is an ordinary [`Page`], so extraction and rendering to Markdown work
//! on it unchanged.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::body::read_text;
use crate::client::{REQUEST_ID_HEADER, excerpt, parse_api_error, transport_error};
use crate::config::Credentials;
use crate::error::{Error, RenderError};
use crate::fetch::{ContentKind, Page};
use crate::output::sanitize;

/// Largest Browser Run response accepted (the HTML arrives JSON-escaped).
pub const MAX_RENDER_BYTES: u64 = 12 * 1024 * 1024;
/// When Browser Run considers the page loaded: no more than two network
/// connections for 500 ms, which lets single-page applications finish.
pub const WAIT_UNTIL: &str = "networkidle2";

/// The JSON body of a render request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRequest<'a> {
    /// The page to load.
    pub url: &'a str,
    /// Page-load behaviour.
    pub goto_options: GotoOptions,
}

/// The `gotoOptions` object of a render request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GotoOptions {
    /// The load event to wait for.
    pub wait_until: &'static str,
}

impl<'a> RenderRequest<'a> {
    /// Builds a request for an already-validated URL.
    #[must_use]
    pub const fn new(url: &'a str) -> Self {
        Self {
            url,
            goto_options: GotoOptions {
                wait_until: WAIT_UNTIL,
            },
        }
    }
}

/// Decodes a Browser Run response into the rendered HTML.
///
/// The success envelope observed live is `{"success":true,"result":"<html>"}`.
///
/// # Errors
/// [`Error::Render`] for a non-2xx status or a body that declares failure,
/// [`Error::UnexpectedResponse`] if a 2xx body is not that envelope.
pub fn parse_rendered(
    status: u16,
    body: &str,
    header_request_id: Option<&str>,
) -> Result<String, Error> {
    let failure = || {
        Error::Render(RenderError(parse_api_error(
            status,
            body,
            header_request_id.map(str::to_owned),
        )))
    };
    if !(200..300).contains(&status) {
        return Err(failure());
    }
    let json: Value = serde_json::from_str(body)
        .map_err(|_| Error::UnexpectedResponse(format!("body is not JSON: {}", excerpt(body))))?;
    if json.get("success") == Some(&Value::Bool(false)) {
        return Err(failure());
    }
    match json.get("result") {
        Some(Value::String(html)) => Ok(html.clone()),
        _ => Err(Error::UnexpectedResponse(format!(
            "no rendered HTML in response: {}",
            excerpt(body)
        ))),
    }
}

/// A reusable HTTP client for the Browser Run content endpoint.
#[derive(Debug, Clone)]
pub struct Renderer {
    agent: ureq::Agent,
    base_url: String,
}

impl Renderer {
    /// Creates a renderer. Redirects are never followed, so the bearer token
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

    /// The endpoint URL for an account.
    #[must_use]
    pub fn endpoint(&self, account_id: &str) -> String {
        format!(
            "{}/accounts/{account_id}/browser-run/content",
            self.base_url
        )
    }

    /// Renders `url` in a browser. Makes exactly one HTTP request, to Cloudflare.
    ///
    /// # Errors
    /// [`Error::Transport`] when no response arrives, [`Error::Render`] when
    /// Browser Run reports a failure, [`Error::UnexpectedResponse`] for
    /// undecodable responses.
    pub fn render(&self, credentials: &Credentials, url: &str) -> Result<Page, Error> {
        let body = serde_json::to_string(&RenderRequest::new(url))
            .map_err(|e| Error::UnexpectedResponse(format!("cannot encode request: {e}")))?;
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
        let text = read_text(&mut response, MAX_RENDER_BYTES).map_err(|e| transport_error(&e))?;
        let html = parse_rendered(status, &text, header_request_id.as_deref())?;
        Ok(Page {
            url: url.to_owned(),
            final_url: url.to_owned(),
            status,
            content_type: Some("text/html".to_owned()),
            kind: ContentKind::Html,
            body: html,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn request_asks_for_the_url_and_waits_for_the_network() {
        assert_eq!(
            serde_json::to_value(RenderRequest::new("https://a.example/x")).unwrap(),
            json!({ "url": "https://a.example/x", "gotoOptions": { "waitUntil": "networkidle2" } })
        );
    }

    #[test]
    fn endpoint_is_the_browser_run_content_path() {
        let renderer = Renderer::new(
            "https://api.cloudflare.com/client/v4/",
            Duration::from_secs(1),
        );
        assert_eq!(
            renderer.endpoint("abc"),
            "https://api.cloudflare.com/client/v4/accounts/abc/browser-run/content"
        );
    }

    #[test]
    fn parses_observed_success_envelope() {
        // Shape captured from the live API on 2026-10-03 (HTML shortened).
        let body = r#"{"success":true,"result":"<!DOCTYPE html><html lang=\"en\"><head><title>Example Domain</title></head><body><p>Hi</p></body></html>"}"#;
        let html = parse_rendered(200, body, None).unwrap();
        assert!(html.starts_with("<!DOCTYPE html><html lang=\"en\">"));
    }

    #[test]
    fn failures_become_render_errors_with_browser_run_hints() {
        let body =
            r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}]}"#;
        for status in [403, 200] {
            let error = parse_rendered(status, body, Some("req-1")).unwrap_err();
            let text = error.to_string();
            assert!(matches!(error, Error::Render(_)), "{text}");
            assert!(text.contains("[10000] Authentication error"), "{text}");
        }
        let text = parse_rendered(403, body, None).unwrap_err().to_string();
        assert!(text.contains("Browser Rendering > Edit"), "{text}");
        assert!(!text.contains("AI Gateway"), "{text}");
    }

    #[test]
    fn success_bodies_without_html_are_unexpected() {
        for body in [
            "",
            "<html>",
            "{}",
            r#"{"success":true}"#,
            r#"{"success":true,"result":{}}"#,
        ] {
            let error = parse_rendered(200, body, None).unwrap_err();
            assert!(
                matches!(error, Error::UnexpectedResponse(_)),
                "{body}: {error}"
            );
        }
    }
}
