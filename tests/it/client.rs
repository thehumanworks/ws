//! The HTTP client against a mock server: wire format, errors and safety limits.

use std::time::Duration;

use crate::common::{Canned, MockServer};
use serde_json::{Value, json};
use ws::client::{Client, MAX_BODY_BYTES, SearchRequest};
use ws::config::Credentials;
use ws::error::Error;
use ws::provider::Provider;

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const TOKEN: &str = "test-token-do-not-leak";

fn credentials() -> Credentials {
    Credentials {
        account_id: ACCOUNT.to_owned(),
        token: TOKEN.to_owned(),
    }
}

fn search(
    server: &MockServer,
    request: &SearchRequest<'_>,
) -> Result<ws::client::SearchOutcome, Error> {
    Client::new(&server.url, Duration::from_secs(5)).search(&credentials(), request)
}

fn ok_body() -> String {
    json!({
        "items": [{ "url": "https://workers.cloudflare.com", "title": "Workers", "description": "Serverless" }],
        "metadata": { "query": "q", "requestId": "meta-id", "latencyMs": 12.5 }
    })
    .to_string()
}

#[test]
fn sends_the_documented_request() {
    let server = MockServer::start(vec![Canned::json(200, &ok_body())]);
    let request = SearchRequest::new(
        "What is Cloudflare Workers?",
        Provider::Linkup,
        3,
        None,
        "my-gw",
    );
    let outcome = search(&server, &request).unwrap();

    let received = server.received();
    assert_eq!(received.len(), 1);
    let sent = &received[0];
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, format!("/accounts/{ACCOUNT}/ai/websearch/"));
    assert_eq!(
        sent.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert!(sent.header("user-agent").unwrap().starts_with("ws/"));
    assert_eq!(
        serde_json::from_str::<Value>(&sent.body).unwrap(),
        json!({
            "query": "What is Cloudflare Workers?",
            "provider": "linkup",
            "limit": 3,
            "options": { "gateway": { "id": "my-gw" } }
        })
    );

    assert_eq!(outcome.response.items[0].title, "Workers");
    assert_eq!(outcome.request_id.as_deref(), Some("meta-id"));
}

#[test]
fn sends_byok_alias_when_given_and_unicode_query_intact() {
    let server = MockServer::start(vec![Canned::json(200, r#"{"items":[]}"#)]);
    let request = SearchRequest::new(
        "crème brûlée 🦀",
        Provider::Exa,
        10,
        Some("team-key"),
        "default",
    );
    let outcome = search(&server, &request).unwrap();
    assert!(outcome.response.items.is_empty());

    let body: Value = serde_json::from_str(&server.received()[0].body).unwrap();
    assert_eq!(body["byokAlias"], "team-key");
    assert_eq!(body["query"], "crème brûlée 🦀");
}

#[test]
fn request_id_falls_back_to_header() {
    let server = MockServer::start(vec![
        Canned::json(200, r#"{"items":[]}"#).header("cf-aig-request-id", "header-id"),
    ]);
    let outcome = search(
        &server,
        &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
    )
    .unwrap();
    assert_eq!(outcome.request_id.as_deref(), Some("header-id"));
}

#[test]
fn error_statuses_become_api_errors() {
    let payment = r#"{"ok":false,"error":{"category":"gateway","code":"web_search_payment_required","status":402,"retryable":false,"gatewayRequestId":"gw-req"}}"#;
    let invalid = r#"{"success":false,"errors":[{"code":7000,"message":"Invalid web search request body"}],"messages":[{"message":"Required","path":["options","gateway","id"]}],"result":null}"#;
    let cases = [
        (400, invalid, Some("7000")),
        (
            401,
            r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}]}"#,
            Some("10000"),
        ),
        (402, payment, Some("web_search_payment_required")),
        (
            403,
            r#"{"success":false,"errors":[{"code":9109,"message":"Forbidden"}]}"#,
            Some("9109"),
        ),
        (
            429,
            r#"{"success":false,"errors":[{"code":971,"message":"Slow down"}]}"#,
            Some("971"),
        ),
        (500, "<html><body>Internal error</body></html>", None),
        (503, "", None),
    ];
    for (status, body, code) in cases {
        let server = MockServer::start(vec![
            Canned::json(status, body),
            Canned::json(200, &ok_body()),
        ]);
        let err = search(
            &server,
            &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
        )
        .unwrap_err();
        let Error::Api(api) = &err else {
            panic!("{status}: expected Api error, got {err}")
        };
        assert_eq!(api.status, status);
        assert_eq!(api.code.as_deref(), code, "{status}");
        assert!(!err.to_string().contains(TOKEN));
        // Paid POSTs are never retried.
        assert_eq!(server.received().len(), 1, "{status} must not be retried");
    }
}

#[test]
fn api_error_details_and_request_id_are_reported() {
    let invalid = r#"{"success":false,"errors":[{"code":7000,"message":"Invalid web search request body"}],"messages":[{"message":"Required","path":["options","gateway","id"]}],"result":null}"#;
    let server = MockServer::start(vec![
        Canned::json(400, invalid).header("cf-aig-request-id", "hdr-1"),
    ]);
    let err = search(
        &server,
        &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
    )
    .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("HTTP 400 [7000] Invalid web search request body"),
        "{text}"
    );
    assert!(text.contains("options.gateway.id: Required"), "{text}");
    assert!(text.contains("request id: hdr-1"), "{text}");
}

#[test]
fn malformed_success_bodies_are_unexpected_responses() {
    for body in ["not json", "{}", r#"{"items":"nope"}"#, "<html></html>"] {
        let server = MockServer::start(vec![Canned::json(200, body)]);
        let err = search(
            &server,
            &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
        )
        .unwrap_err();
        assert!(matches!(err, Error::UnexpectedResponse(_)), "{body}: {err}");
    }
}

#[test]
fn redirects_are_not_followed() {
    let elsewhere = MockServer::start(vec![Canned::json(200, &ok_body())]);
    let server = MockServer::start(vec![
        Canned::json(307, "").header("Location", &format!("{}/steal", elsewhere.url)),
    ]);
    let err = search(
        &server,
        &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
    )
    .unwrap_err();
    assert!(matches!(err, Error::Api(_) | Error::Transport(_)), "{err}");
    assert_eq!(server.received().len(), 1);
    assert!(
        elsewhere.received().is_empty(),
        "the token must never reach the redirect target"
    );
}

#[test]
fn oversized_bodies_are_rejected() {
    let padding = "x".repeat(usize::try_from(MAX_BODY_BYTES).unwrap() + 1024);
    let body = format!(r#"{{"items":[{{"url":"u","title":"{padding}"}}]}}"#);
    let server = MockServer::start(vec![Canned::json(200, &body)]);
    let err = search(
        &server,
        &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
    )
    .unwrap_err();
    assert!(
        matches!(&err, Error::Transport(m) if m.contains("safety limit")),
        "{err}"
    );
}

#[test]
fn slow_responses_time_out() {
    let server = MockServer::start(vec![
        Canned::json(200, &ok_body()).delayed(Duration::from_secs(3)),
    ]);
    let client = Client::new(&server.url, Duration::from_secs(1));
    let err = client
        .search(
            &credentials(),
            &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
        )
        .unwrap_err();
    assert!(
        matches!(&err, Error::Transport(m) if m.contains("timed out")),
        "{err}"
    );
}

#[test]
fn connection_refused_is_a_transport_error() {
    // Bind then drop to get a port nothing is listening on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let client = Client::new(&format!("http://127.0.0.1:{port}"), Duration::from_secs(2));
    let err = client
        .search(
            &credentials(),
            &SearchRequest::new("q", Provider::Ceramic, 1, None, "default"),
        )
        .unwrap_err();
    assert!(matches!(err, Error::Transport(_)), "{err}");
    assert_eq!(err.exit_code(), 1);
}
