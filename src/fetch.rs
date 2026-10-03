//! Fetching one web page for `ws fetch`.
//!
//! This is deliberately separate from [`crate::client`]: a page can live on any
//! host, so nothing here knows about Cloudflare credentials and no secret is
//! ever attached to a request. That is also why redirects are followed here
//! and never in the search client. Unless told otherwise it only talks to
//! public addresses, checked after DNS resolution and again on every redirect. The module only retrieves text; turning
//! HTML into Markdown is [`crate::markdown`]'s job, so other consumers (such as
//! structured scraping) can be built on the same [`Page`].

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use ureq::ResponseExt;
use ureq::config::Config;
use ureq::http::Uri;
// `unversioned` is ureq's extension API and may change in a minor release;
// Cargo.lock pins the version this was written against.
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

use crate::body::read_text;
use crate::error::Error;
use crate::output::sanitize;
use crate::validate::ip_is_public;

/// Largest page body accepted (after decompression), as local protection
/// against runaway responses.
pub const MAX_PAGE_BYTES: u64 = 5 * 1024 * 1024;
/// Most redirects followed before giving up.
pub const MAX_REDIRECTS: u32 = 10;
/// `Accept` header asking for Markdown where a site offers it, else HTML.
pub const ACCEPT_MARKDOWN: &str = "text/markdown, text/html;q=0.9, text/plain;q=0.8, */*;q=0.1";
/// `Accept` header asking for HTML.
pub const ACCEPT_HTML: &str = "text/html, application/xhtml+xml;q=0.9, */*;q=0.1";

/// What kind of text a page body is, decided from its `Content-Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    /// HTML or XHTML: convertible to Markdown.
    Html,
    /// Already Markdown (the site honoured `Accept: text/markdown`).
    Markdown,
    /// Other text (plain text, JSON, XML, ...), shown as served.
    Text,
}

/// A fetched page: what was asked for, what answered, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The URL that was requested.
    pub url: String,
    /// The URL that answered, after redirects.
    pub final_url: String,
    /// HTTP status of the final response (always 2xx).
    pub status: u16,
    /// The `Content-Type` header, sanitised, when the server sent one.
    pub content_type: Option<String>,
    /// How the body should be treated.
    pub kind: ContentKind,
    /// The body decoded to UTF-8, exactly as served otherwise.
    pub body: String,
}

/// Classifies a `Content-Type` header value. `None` means the content is not
/// text `ws` can show (an image, a PDF, an archive, ...).
#[must_use]
pub fn kind_of(content_type: &str) -> Option<ContentKind> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "text/html" | "application/xhtml+xml" => Some(ContentKind::Html),
        "text/markdown" | "text/x-markdown" => Some(ContentKind::Markdown),
        "application/json" | "application/xml" | "application/javascript" => {
            Some(ContentKind::Text)
        }
        other => {
            (other.starts_with("text/") || other.ends_with("+json") || other.ends_with("+xml"))
                .then_some(ContentKind::Text)
        }
    }
}

/// Guesses the kind of a body served without a `Content-Type`.
#[must_use]
pub fn sniff(body: &str) -> ContentKind {
    let start: String = body
        .trim_start()
        .chars()
        .take(15)
        .collect::<String>()
        .to_ascii_lowercase();
    if start.starts_with("<!doctype html") || start.starts_with("<html") {
        ContentKind::Html
    } else {
        ContentKind::Text
    }
}

/// Which hosts a [`Fetcher`] may connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Only addresses on the public internet (see [`ip_is_public`]).
    PublicOnly,
    /// Any address, including loopback and private networks.
    AllowPrivate,
}

/// Why the resolver refused a host.
#[derive(Debug)]
struct NotPublic {
    host: String,
    ip: IpAddr,
}

impl fmt::Display for NotPublic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} resolves to {}, which is not a public address; \
             pass --allow-private to fetch it anyway",
            self.host, self.ip
        )
    }
}

impl std::error::Error for NotPublic {}

/// A resolver that refuses any host with a non-public address. Because it sits
/// below the HTTP layer it also covers every redirect and DNS names that point
/// at internal addresses.
#[derive(Debug, Default)]
struct PublicOnlyResolver(DefaultResolver);

impl Resolver for PublicOnlyResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addresses = self.0.resolve(uri, config, timeout)?;
        if let Some(address) = addresses.iter().find(|a| !ip_is_public(a.ip())) {
            return Err(ureq::Error::Other(Box::new(NotPublic {
                host: uri.host().unwrap_or_default().to_owned(),
                ip: address.ip(),
            })));
        }
        Ok(addresses)
    }
}

/// A reusable HTTP client for reading public web pages. It holds no secrets.
#[derive(Debug, Clone)]
pub struct Fetcher {
    agent: ureq::Agent,
}

impl Fetcher {
    /// Creates a fetcher with an overall deadline per page (redirects included).
    #[must_use]
    pub fn new(timeout: Duration, access: Access) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(MAX_REDIRECTS)
            .http_status_as_error(false)
            .user_agent(concat!("ws/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = match access {
            Access::PublicOnly => ureq::Agent::with_parts(
                config,
                DefaultConnector::default(),
                PublicOnlyResolver::default(),
            ),
            Access::AllowPrivate => config.into(),
        };
        Self { agent }
    }

    /// Fetches `url` with a single `GET`, following redirects.
    ///
    /// `accept` is sent as the `Accept` header; see [`ACCEPT_MARKDOWN`] and
    /// [`ACCEPT_HTML`]. The URL must already be validated.
    ///
    /// # Errors
    /// [`Error::Transport`] when no response arrives or the body is too large,
    /// [`Error::PageBlocked`] when a host is not public, [`Error::PageStatus`] for a non-2xx answer, [`Error::PageContent`] when
    /// the page is not text.
    pub fn get(&self, url: &str, accept: &str) -> Result<Page, Error> {
        let mut response = self
            .agent
            .get(url)
            .header("Accept", accept)
            .call()
            .map_err(|e| transport_error(&e))?;

        let status = response.status().as_u16();
        let final_url = sanitize(&response.get_uri().to_string());
        if !(200..300).contains(&status) {
            return Err(Error::PageStatus {
                status,
                url: final_url,
            });
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(sanitize);
        // Decide before reading, so a large binary is never downloaded.
        let declared = match content_type.as_deref() {
            Some(value) => Some(kind_of(value).ok_or_else(|| {
                Error::PageContent(format!(
                    "{final_url} is '{value}', not text; ws fetch reads HTML, Markdown and plain text"
                ))
            })?),
            None => None,
        };
        let body = read_text(&mut response, MAX_PAGE_BYTES).map_err(|e| transport_error(&e))?;
        let kind = declared.unwrap_or_else(|| sniff(&body));
        Ok(Page {
            url: url.to_owned(),
            final_url,
            status,
            content_type,
            kind,
            body,
        })
    }
}

fn transport_error(error: &ureq::Error) -> Error {
    if let ureq::Error::Other(inner) = error
        && let Some(blocked) = inner.downcast_ref::<NotPublic>()
    {
        return Error::PageBlocked(sanitize(&blocked.to_string()));
    }
    Error::Transport(if matches!(error, ureq::Error::Timeout(_)) {
        "timed out fetching the page (see --timeout)".to_owned()
    } else if let ureq::Error::BodyExceedsLimit(limit) = error {
        format!("page exceeds the {limit}-byte safety limit")
    } else {
        sanitize(&error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_are_classified() {
        for html in [
            "text/html",
            "text/html; charset=utf-8",
            "TEXT/HTML;charset=ISO-8859-1",
            "application/xhtml+xml",
        ] {
            assert_eq!(kind_of(html), Some(ContentKind::Html), "{html}");
        }
        assert_eq!(
            kind_of("text/markdown; charset=utf-8"),
            Some(ContentKind::Markdown)
        );
        for text in [
            "text/plain",
            "application/json",
            "application/ld+json",
            "application/rss+xml; charset=utf-8",
            "text/csv",
        ] {
            assert_eq!(kind_of(text), Some(ContentKind::Text), "{text}");
        }
        for binary in [
            "application/pdf",
            "image/png",
            "application/octet-stream",
            "",
        ] {
            assert_eq!(kind_of(binary), None, "{binary}");
        }
    }

    #[test]
    fn bodies_without_a_content_type_are_sniffed() {
        assert_eq!(sniff("  \n<!DOCTYPE html><html>"), ContentKind::Html);
        assert_eq!(sniff("<HTML lang=en>"), ContentKind::Html);
        assert_eq!(sniff("# heading"), ContentKind::Text);
        assert_eq!(sniff(""), ContentKind::Text);
        assert_eq!(sniff("<htmé"), ContentKind::Text);
    }
}
