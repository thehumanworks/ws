//! End-to-end tests of `ws fetch` against the mock server: formats, redirects,
//! failures, and the guarantee that no credential ever reaches a fetched host.

use std::collections::HashMap;
use std::time::Duration;

use crate::common::{Canned, MockServer};
use assert_cmd::Command;
use serde_json::{Value, json};
use ws::fetch::{ACCEPT_HTML, ACCEPT_MARKDOWN, Access, ContentKind, Fetcher, MAX_PAGE_BYTES};

const TOKEN: &str = "test-token-do-not-leak";
const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const HTML: &str = "<!doctype html><html><head><title>Mock &amp; Page</title>\
    <script>var tracker = 1;</script></head><body><nav><a href=\"/\">Home</a></nav>\
    <main><h1>Heading</h1>\
    <p>Some <em>text</em> and a <a href=\"https://a.example/\">link</a>.</p></main>\
    <footer>Copyright Mock</footer></body></html>";
const MAIN_MARKDOWN: &str = "# Heading\n\nSome *text* and a [link](https://a.example/).";

struct Run {
    code: u8,
    stdout: String,
    stderr: String,
}

/// Runs `ws` with `--allow-private`, since the mock server is on loopback.
fn run(args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut full = args.to_vec();
    if args.first() == Some(&"fetch") {
        full.push("--allow-private");
    }
    run_exact(&full, env)
}

fn run_exact(args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut map: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let _ = map
        .entry("WS_BACKEND".to_owned())
        .or_insert_with(|| "cloudflare".to_owned());
    let lookup = move |key: &str| map.get(key).cloned();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let full = std::iter::once("ws").chain(args.iter().copied());
    let code = ws::cli::run(full, &lookup, &mut out, &mut err);
    Run {
        code,
        stdout: String::from_utf8(out).unwrap(),
        stderr: String::from_utf8(err).unwrap(),
    }
}

fn html_page() -> Canned {
    Canned::page(200, "text/html; charset=utf-8", HTML.as_bytes())
}

#[test]
fn fetch_prints_markdown_by_default_without_any_credentials() {
    let server = MockServer::start(vec![html_page()]);
    let url = format!("{}/docs/page?x=1", server.url);
    let out = run(&["fetch", &url], &[]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, format!("{MAIN_MARKDOWN}\n"));
    assert_eq!(out.stderr, "");

    let received = server.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].method, "GET");
    assert_eq!(received[0].path, "/docs/page?x=1");
    assert_eq!(received[0].header("accept"), Some(ACCEPT_MARKDOWN));
    assert!(received[0].header("user-agent").unwrap().starts_with("ws/"));
}

#[test]
fn fetch_never_sends_cloudflare_credentials() {
    let server = MockServer::start(vec![html_page()]);
    let out = run(
        &["fetch", &server.url],
        &[
            ("CLOUDFLARE_ACCOUNT_ID", "0123456789abcdef0123456789abcdef"),
            ("CLOUDFLARE_API_TOKEN", TOKEN),
        ],
    );

    assert_eq!(out.code, 0, "{}", out.stderr);
    let received = server.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].header("authorization"), None);
    for (name, value) in &received[0].headers {
        assert!(!value.contains(TOKEN), "{name} leaks the token");
    }
    assert!(!received[0].path.contains(TOKEN));
    assert_eq!(received[0].body, "");
}

#[test]
fn raw_keeps_the_whole_page_in_every_format() {
    let server = MockServer::start(vec![html_page(), html_page(), html_page()]);

    let markdown = run(&["fetch", &server.url, "--raw"], &[]);
    assert_eq!(markdown.code, 0, "{}", markdown.stderr);
    assert_eq!(
        markdown.stdout,
        format!("[Home](/)\n\n{MAIN_MARKDOWN}\n\nCopyright Mock\n")
    );

    let html = run(&["fetch", &server.url, "--format", "html", "--raw"], &[]);
    assert_eq!(html.code, 0, "{}", html.stderr);
    assert_eq!(html.stdout, format!("{HTML}\n"), "as served, byte for byte");

    let json = run(&["fetch", &server.url, "--format", "json", "--raw"], &[]);
    let value: Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(value["extracted"], false);
    assert!(
        value["markdown"]
            .as_str()
            .unwrap()
            .contains("Copyright Mock")
    );
}

#[test]
fn format_html_prints_only_the_main_content_by_default() {
    let server = MockServer::start(vec![html_page()]);
    let out = run(&["fetch", &server.url, "--format", "html"], &[]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        "<h1>Heading</h1><p>Some <em>text</em> and a <a href=\"https://a.example/\">link</a>.</p>\n"
    );
    assert_eq!(server.received()[0].header("accept"), Some(ACCEPT_HTML));
}

#[test]
fn nearly_empty_pages_get_a_note_on_stderr_only() {
    let shell =
        "<html><body><div id=\"root\"></div><script src=\"/app.js\"></script></body></html>";
    let server = MockServer::start(vec![
        Canned::page(200, "text/html", shell.as_bytes()),
        Canned::page(200, "text/html", shell.as_bytes()),
    ]);
    let out = run(&["fetch", &server.url], &[]);
    assert_eq!(out.code, 0);
    assert_eq!(out.stdout, "\n");
    assert_eq!(
        out.stderr,
        "ws: note: almost no readable text was found; try --raw to keep the whole page, \
         or --render if it needs JavaScript\n"
    );
    let raw = run(&["fetch", &server.url, "--raw", "-f", "json"], &[]);
    assert_eq!(raw.code, 0);
    assert!(
        raw.stderr
            .ends_with("try --render if it needs JavaScript\n"),
        "{}",
        raw.stderr
    );
    let _: Value = serde_json::from_str(&raw.stdout).unwrap();
}

#[test]
fn format_json_is_a_single_parseable_object() {
    let server = MockServer::start(vec![html_page()]);
    let out = run(&["fetch", &server.url, "-f", "json"], &[]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        value,
        json!({
            "url": server.url,
            "finalUrl": format!("{}/", server.url),
            "status": 200,
            "contentType": "text/html; charset=utf-8",
            "title": "Mock & Page",
            "extracted": true,
            "rendered": false,
            "markdown": MAIN_MARKDOWN
        })
    );
}

#[test]
fn markdown_and_plain_text_responses_pass_through_unconverted() {
    let body = "# Served as Markdown\n\n<b>not converted</b>\n";
    for content_type in ["text/markdown; charset=utf-8", "text/plain", ""] {
        let server = MockServer::start(vec![Canned::page(200, content_type, body.as_bytes())]);
        let out = run(&["fetch", &server.url], &[]);
        assert_eq!(out.code, 0, "{content_type}: {}", out.stderr);
        assert_eq!(out.stdout, body, "{content_type}");
        assert_eq!(out.stderr, "");
    }
}

#[test]
fn html_without_a_content_type_is_still_converted() {
    let server = MockServer::start(vec![Canned::page(200, "", HTML.as_bytes())]);
    let out = run(&["fetch", &server.url], &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.starts_with("# Heading\n"), "{}", out.stdout);
}

#[test]
fn redirects_are_followed_and_the_final_url_reported() {
    let server = MockServer::start(vec![
        Canned::page(301, "text/plain", b"moved").header("Location", "/new/place"),
        html_page(),
    ]);
    let start = format!("{}/old", server.url);
    let out = run(&["fetch", &start, "--format", "json"], &[]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["url"], start);
    assert_eq!(value["finalUrl"], format!("{}/new/place", server.url));
    let paths: Vec<String> = server.received().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, ["/old", "/new/place"]);
}

#[test]
fn declared_charset_is_decoded_to_utf8() {
    // "café" in ISO-8859-1: the é is the single byte 0xE9.
    let body = b"<html><body><p>caf\xe9</p></body></html>";
    let server = MockServer::start(vec![Canned::page(
        200,
        "text/html; charset=ISO-8859-1",
        body,
    )]);
    let out = run(&["fetch", &server.url], &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "café\n");
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn gzip_is_requested_and_decoded() {
    let server = MockServer::start(vec![
        Canned::page(200, "text/html", &gzip(HTML.as_bytes())).header("Content-Encoding", "gzip"),
    ]);
    let out = run(&["fetch", &server.url], &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, format!("{MAIN_MARKDOWN}\n"));
    assert!(
        server.received()[0]
            .header("accept-encoding")
            .unwrap()
            .contains("gzip")
    );
}

#[test]
fn the_size_cap_applies_after_decompression() {
    let size = usize::try_from(MAX_PAGE_BYTES).unwrap() + 1;
    let bomb = gzip(&vec![b'a'; size]);
    assert!(bomb.len() < 64 * 1024, "the compressed body is small");
    let server = MockServer::start(vec![
        Canned::page(200, "text/html", &bomb).header("Content-Encoding", "gzip"),
    ]);
    let out = run(&["fetch", &server.url], &[]);
    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("safety limit"), "{}", out.stderr);
}

#[test]
fn non_public_hosts_are_refused_unless_allowed() {
    let server = MockServer::start(vec![html_page()]);
    let port = server.url.rsplit(':').next().unwrap();
    for url in [
        server.url.clone(),
        format!("http://localhost:{port}/"),
        format!("http://[::1]:{port}/"),
        format!("http://[::ffff:127.0.0.1]:{port}/"),
        "http://10.0.0.1/".to_owned(),
        "http://169.254.169.254/latest/meta-data/".to_owned(),
        "http://0.0.0.0/".to_owned(),
    ] {
        let out = run_exact(&["fetch", &url], &[]);
        assert_eq!(out.code, 1, "{url}");
        assert_eq!(out.stdout, "");
        assert!(
            out.stderr.starts_with("ws: blocked: "),
            "{url}: {}",
            out.stderr
        );
        assert!(
            out.stderr.contains("not a public address"),
            "{}",
            out.stderr
        );
        assert!(out.stderr.contains("--allow-private"), "{}", out.stderr);
    }
    assert!(server.received().is_empty(), "no connection may be made");

    let fetcher = Fetcher::new(Duration::from_secs(5), Access::PublicOnly);
    let error = fetcher.get(&server.url, ACCEPT_HTML).unwrap_err();
    assert!(matches!(error, ws::error::Error::PageBlocked(_)), "{error}");
}

// --- --render: Cloudflare Browser Run ------------------------------------------

fn render_env(server: &MockServer) -> Vec<(&str, &str)> {
    vec![
        ("CLOUDFLARE_ACCOUNT_ID", ACCOUNT),
        ("CLOUDFLARE_API_TOKEN", TOKEN),
        ("WS_API_BASE_URL", server.url.as_str()),
    ]
}

fn rendered() -> Canned {
    Canned::json(200, &json!({ "success": true, "result": HTML }).to_string())
}

#[test]
fn render_asks_browser_run_and_extracts_the_result() {
    let server = MockServer::start(vec![rendered()]);
    let out = run_exact(
        &["fetch", "https://spa.example/app", "--render", "-f", "json"],
        &render_env(&server),
    );

    assert_eq!(out.code, 0, "{}", out.stderr);
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        value,
        json!({
            "url": "https://spa.example/app",
            "finalUrl": "https://spa.example/app",
            "status": 200,
            "contentType": "text/html",
            "title": "Mock & Page",
            "extracted": true,
            "rendered": true,
            "markdown": MAIN_MARKDOWN
        })
    );

    let received = server.received();
    assert_eq!(received.len(), 1, "exactly one request, to Cloudflare");
    assert_eq!(received[0].method, "POST");
    assert_eq!(
        received[0].path,
        format!("/accounts/{ACCOUNT}/browser-run/content")
    );
    assert_eq!(
        received[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(
        serde_json::from_str::<Value>(&received[0].body).unwrap(),
        json!({ "url": "https://spa.example/app", "gotoOptions": { "waitUntil": "networkidle2" } })
    );
}

#[test]
fn render_needs_credentials_and_sends_nothing_without_them() {
    let server = MockServer::start(vec![rendered()]);
    let out = run_exact(
        &["fetch", "https://spa.example/", "--render"],
        &[("WS_API_BASE_URL", server.url.as_str())],
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("CLOUDFLARE_ACCOUNT_ID is not set"),
        "{}",
        out.stderr
    );

    let conflict = run_exact(
        &[
            "fetch",
            "https://spa.example/",
            "--render",
            "--allow-private",
        ],
        &render_env(&server),
    );
    assert_eq!(conflict.code, 2);

    let mut env = render_env(&server);
    env[2] = ("WS_API_BASE_URL", "https://evil.example");
    assert_eq!(
        run_exact(&["fetch", "https://spa.example/", "--render"], &env).code,
        2
    );

    let invalid = run_exact(&["fetch", "spa.example", "--render"], &render_env(&server));
    assert_eq!(invalid.code, 2);
    assert!(server.received().is_empty(), "nothing may be sent");
}

#[test]
fn render_failures_exit_1_with_a_browser_run_hint_and_no_token() {
    let body = r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}]}"#;
    let server = MockServer::start(vec![Canned::json(403, body)]);
    let out = run_exact(
        &["fetch", "https://spa.example/", "--render"],
        &render_env(&server),
    );

    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr
            .starts_with("ws: Cloudflare Browser Run error: HTTP 403 [10000] Authentication error"),
        "{}",
        out.stderr
    );
    assert!(
        out.stderr.contains("Browser Rendering > Edit"),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains(TOKEN));
}

#[test]
fn control_characters_in_a_page_cannot_drive_the_terminal() {
    let body = "<p>before\u{1b}]0;pwned\u{7}after</p>";
    for format in ["markdown", "html"] {
        let server = MockServer::start(vec![Canned::page(200, "text/html", body.as_bytes())]);
        let out = run(&["fetch", &server.url, "--format", format], &[]);
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert!(!out.stdout.contains('\u{1b}'), "{format}");
        assert!(!out.stdout.contains('\u{7}'), "{format}");
        assert!(out.stdout.contains("before"), "{format}");
    }
}

#[test]
fn http_errors_exit_1_and_name_the_status() {
    let server = MockServer::start(vec![Canned::page(404, "text/html", b"<h1>nope</h1>")]);
    let url = format!("{}/missing", server.url);
    let out = run(&["fetch", &url], &[]);

    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        format!("ws: fetch failed: HTTP 404 from {url}\n")
    );
}

#[test]
fn binary_content_is_refused_with_exit_1() {
    let server = MockServer::start(vec![Canned::page(200, "application/pdf", b"%PDF-1.7")]);
    let out = run(&["fetch", &server.url], &[]);

    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr.contains("'application/pdf', not text"),
        "{}",
        out.stderr
    );
}

#[test]
fn invalid_urls_exit_2_without_sending_anything() {
    let server = MockServer::start(vec![html_page()]);
    let spaced = format!("{}/a b", server.url);
    let port = server.url.rsplit(':').next().unwrap();
    let no_scheme = format!("127.0.0.1:{port}");
    for bad in [
        spaced.as_str(),
        no_scheme.as_str(),
        "file:///etc/passwd",
        "ftp://a.example",
    ] {
        let out = run(&["fetch", bad], &[]);
        assert_eq!(out.code, 2, "{bad}");
        assert!(
            out.stderr.starts_with("ws: invalid input: URL"),
            "{}",
            out.stderr
        );
        assert_eq!(out.stdout, "");
    }
    for args in [
        &["fetch"][..],
        &["fetch", &server.url, "--format", "pdf"],
        &["fetch", &server.url, "--timeout", "0"],
        &["fetch", &server.url, "--timeout", "301"],
    ] {
        assert_eq!(run(args, &[]).code, 2, "{args:?}");
    }
    let out = run(&["fetch", &server.url], &[("WS_TIMEOUT_SECS", "soon")]);
    assert_eq!(out.code, 2);
    assert!(server.received().is_empty(), "nothing may be sent");
}

#[test]
fn timeout_comes_from_the_flag_or_environment_and_exits_1() {
    for (args, env) in [
        (&["--timeout", "1"][..], &[][..]),
        (&[][..], &[("WS_TIMEOUT_SECS", "1")][..]),
    ] {
        let server = MockServer::start(vec![html_page().delayed(Duration::from_millis(2500))]);
        let mut full = vec!["fetch", server.url.as_str()];
        full.extend_from_slice(args);
        let out = run(&full, env);
        assert_eq!(out.code, 1);
        assert!(
            out.stderr.contains("timed out fetching the page"),
            "{}",
            out.stderr
        );
    }
}

#[test]
fn oversize_pages_are_refused() {
    let size = usize::try_from(MAX_PAGE_BYTES).unwrap() + 1;
    let server = MockServer::start(vec![Canned::page(200, "text/html", &vec![b'a'; size])]);
    let out = run(&["fetch", &server.url], &[]);

    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "");
    assert!(out.stderr.contains("safety limit"), "{}", out.stderr);
}

#[test]
fn connection_failures_exit_1() {
    // Bind then drop a listener so the port is very likely closed.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let out = run(&["fetch", &format!("http://127.0.0.1:{port}/")], &[]);
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.starts_with("ws: request failed: "),
        "{}",
        out.stderr
    );
}

#[test]
fn fetcher_is_usable_as_a_library_and_returns_the_raw_page() {
    let server = MockServer::start(vec![html_page()]);
    let page = Fetcher::new(Duration::from_secs(5), Access::AllowPrivate)
        .get(&server.url, ACCEPT_HTML)
        .unwrap();

    assert_eq!(page.url, server.url);
    assert_eq!(page.status, 200);
    assert_eq!(page.kind, ContentKind::Html);
    assert_eq!(page.body, HTML);
    let document = ws::extract::Document::parse(&page.body).unwrap();
    assert_eq!(document.title().as_deref(), Some("Mock & Page"));
}

#[test]
fn binary_fetches_with_an_empty_environment() {
    let server = MockServer::start(vec![html_page()]);
    let mut command = Command::cargo_bin("ws").unwrap();
    let _: &mut Command = command.env_clear().env("WS_BACKEND", "cloudflare");
    // Windows cannot open sockets without SystemRoot.
    if let Some(root) = std::env::var_os("SystemRoot") {
        let _: &mut Command = command.env("SystemRoot", root);
    }
    let output = command
        .args(["fetch", &server.url, "--allow-private"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{MAIN_MARKDOWN}\n")
    );
    assert_eq!(server.received()[0].header("authorization"), None);
}
