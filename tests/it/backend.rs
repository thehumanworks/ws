//! Runtime selection and the local browser boundary, with no public network.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::{Value, json};
use ws::backend::Backend;
use ws::error::Error;
use ws::fetch::{Access, ContentKind, Page};
use ws::lightpanda::{Browser, Dump, Rendered, Request};

use crate::common::{Canned, MockServer};

const HTML: &str = "<html><head><title>Rendered title</title></head><body><nav>Navigation</nav><main><h1>Rendered content</h1><p>Enough text for a readable page.</p></main></body></html>";
const CARDS: &str = "<main id='mixed-main'><div class='snippet' data-type='web' data-pos='1'><a href='https://result.example/'><span class='search-snippet-title'>Result</span></a><div class='generic-snippet'><div class='content'>Full description</div></div></div></main>";

struct Fake {
    content: Rendered,
    requests: RefCell<Vec<Request>>,
}

impl Fake {
    fn html(html: &str) -> Self {
        Self {
            content: Rendered::Html(Page {
                url: "https://original.example/".to_owned(),
                final_url: "https://final.example/".to_owned(),
                status: 200,
                content_type: Some("text/html".to_owned()),
                kind: ContentKind::Html,
                body: html.to_owned(),
            }),
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl Browser for Fake {
    fn retrieve(&self, request: &Request) -> Result<Rendered, Error> {
        self.requests.borrow_mut().push(request.clone());
        Ok(self.content.clone())
    }
}

fn run(args: &[&str], pairs: &[(&str, &str)], browser: &dyn Browser) -> (u8, Vec<u8>, String) {
    let map = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<HashMap<_, _>>();
    let env = move |key: &str| map.get(key).cloned();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let code = ws::cli::run_with_browser(
        std::iter::once("ws").chain(args.iter().copied()),
        &env,
        &mut stdout,
        &mut stderr,
        browser,
    );
    (code, stdout, String::from_utf8(stderr).unwrap())
}

#[test]
fn backend_flag_env_file_and_platform_default_are_independent_of_provider() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "backend='cloudflare'\nprovider='not-valid'\ngateway_id='not valid'\n",
    )
    .unwrap();
    let file = path.to_str().unwrap();
    let fake = Fake::html(CARDS);
    let cases = [
        (
            vec!["search", "query", "--backend", "lightpanda"],
            vec![("WS_CONFIG", file), ("WS_BACKEND", "cloudflare")],
        ),
        (
            vec!["search", "query"],
            vec![("WS_CONFIG", file), ("WS_BACKEND", "lightpanda")],
        ),
    ];
    for (args, env) in cases {
        let (code, _, error) = run(&args, &env, &fake);
        assert_eq!(code, 0, "{error}");
    }
    std::fs::write(&path, "backend='lightpanda'\nprovider='invalid'\n").unwrap();
    assert_eq!(
        run(&["search", "query"], &[("WS_CONFIG", file)], &fake).0,
        0
    );
    let default = run(&["search", "query"], &[], &fake).0;
    assert_eq!(
        default,
        if Backend::platform_default() == Backend::Lightpanda {
            0
        } else {
            2
        }
    );
    assert!(
        fake.requests
            .borrow()
            .iter()
            .all(|request| request.access == Access::PublicOnly)
    );
}

#[test]
fn local_search_ignores_unrelated_malformed_cloudflare_environment() {
    let fake = Fake::html(CARDS);
    let (code, stdout, error) = run(
        &[
            "search",
            "rust &é",
            "--backend",
            "lightpanda",
            "--json",
            "--limit",
            "1",
        ],
        &[
            ("CLOUDFLARE_API_TOKEN", "bad token"),
            ("CLOUDFLARE_ACCOUNT_ID", "bad"),
            ("WS_PROVIDER", "bad"),
            ("WS_GATEWAY_ID", ""),
            ("WS_BYOK_ALIAS", "bad alias"),
            ("WS_API_BASE_URL", "bad"),
        ],
        &fake,
    );
    assert_eq!(code, 0, "{error}");
    let output: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(output["backend"], "lightpanda");
    assert_eq!(output["provider"], "brave");
    assert_eq!(output["items"].as_array().unwrap().len(), 1);
    assert!(output.get("gateway").is_none());
    assert_eq!(
        fake.requests.borrow()[0].url,
        "https://search.brave.com/search?q=rust%20%26%C3%A9"
    );
    let query = "🦀".repeat(1024);
    assert_eq!(
        run(&["search", &query, "--backend", "lightpanda"], &[], &fake).0,
        0
    );
}

#[test]
fn cloudflare_only_flags_png_and_invalid_inputs_fail_before_browser_or_billing() {
    let fake = Fake::html(CARDS);
    for flag in ["--provider", "--gateway", "--account-id", "--byok-alias"] {
        let value = if flag == "--provider" { "exa" } else { "value" };
        let (code, _, error) = run(
            &["search", "query", "--backend", "lightpanda", flag, value],
            &[],
            &fake,
        );
        assert_eq!(code, 2);
        assert!(error.contains("require --backend cloudflare"));
    }
    assert_eq!(
        run(&["search", " ", "--backend", "lightpanda"], &[], &fake).0,
        2
    );
    assert_eq!(
        run(
            &[
                "search",
                "query",
                "--limit",
                "11",
                "--backend",
                "lightpanda"
            ],
            &[],
            &fake
        )
        .0,
        2
    );
    assert_eq!(
        run(&["fetch", "invalid", "--backend", "lightpanda"], &[], &fake).0,
        2
    );
    assert_eq!(
        run(
            &[
                "fetch",
                "https://example.com/",
                "--backend",
                "cloudflare",
                "--format",
                "png",
                "--render"
            ],
            &[],
            &fake
        )
        .0,
        2
    );
    assert!(fake.requests.borrow().is_empty());
}

#[test]
fn rendered_fetch_uses_existing_pipeline_and_raw_html_keeps_navigation() {
    let fake = Fake::html(HTML);
    let (code, stdout, error) = run(
        &[
            "fetch",
            "https://original.example/",
            "--backend",
            "lightpanda",
            "--format",
            "json",
            "--render",
        ],
        &[],
        &fake,
    );
    assert_eq!(code, 0, "{error}");
    let output: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(output["rendered"], true);
    assert_eq!(output["extracted"], true);
    assert_eq!(output["url"], "https://original.example/");
    assert_eq!(output["finalUrl"], "https://final.example/");
    assert!(!output["markdown"].as_str().unwrap().contains("Navigation"));
    let (code, stdout, _) = run(
        &[
            "fetch",
            "https://original.example/",
            "--backend",
            "lightpanda",
            "-f",
            "html",
            "--raw",
        ],
        &[],
        &fake,
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, format!("{HTML}\n").as_bytes());
    assert_eq!(fake.requests.borrow().len(), 2);
}

#[test]
fn png_writes_exact_binary_bytes_even_with_raw_and_render() {
    let bytes = b"\x89PNG\r\n\x1a\n\0\x1b\xff\r\n".to_vec();
    let fake = Fake {
        content: Rendered::Png(bytes.clone()),
        requests: RefCell::new(Vec::new()),
    };
    let (code, stdout, error) = run(
        &[
            "fetch",
            "http://localhost/",
            "--backend",
            "lightpanda",
            "--format",
            "png",
            "--raw",
            "--render",
            "--allow-private",
        ],
        &[],
        &fake,
    );
    assert_eq!(code, 0, "{error}");
    assert_eq!(stdout, bytes);
    assert_eq!(fake.requests.borrow()[0].dump, Dump::Png);
    assert_eq!(fake.requests.borrow()[0].access, Access::AllowPrivate);
}

#[test]
fn config_backend_round_trip_and_show_tolerates_cloudflare_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let path = path.to_str().unwrap();
    let fake = Fake::html(HTML);
    assert_eq!(
        run(
            &["config", "set", "backend", "lightpanda"],
            &[("WS_CONFIG", path)],
            &fake
        )
        .0,
        0
    );
    assert_eq!(
        ws::config::FileConfig::load(std::path::Path::new(path))
            .unwrap()
            .backend,
        Some(Backend::Lightpanda)
    );
    let (code, stdout, _) = run(
        &["config", "show"],
        &[
            ("WS_CONFIG", path),
            ("WS_PROVIDER", "bad"),
            ("WS_GATEWAY_ID", "\x1b[31mbad"),
        ],
        &fake,
    );
    assert_eq!(code, 0);
    assert!(!stdout.contains(&0x1b));
    assert!(
        String::from_utf8(stdout)
            .unwrap()
            .contains("backend      lightpanda (config file)")
    );
    assert_eq!(
        run(
            &["config", "unset", "backend"],
            &[("WS_CONFIG", path)],
            &fake
        )
        .0,
        0
    );
    assert_eq!(
        ws::config::FileConfig::load(std::path::Path::new(path))
            .unwrap()
            .backend,
        None
    );
}

#[test]
fn actual_bundled_browser_executes_js_redirects_subresources_and_png_without_credentials() {
    if Backend::platform_default() != Backend::Lightpanda {
        return;
    }
    let cache = tempfile::tempdir().unwrap();
    let script = "document.querySelector('main').textContent='JavaScript rendered content';";
    let html = "<html><head><title>Native test</title></head><body><nav>Navigation</nav><main>Initial</main><script src='/script.js'></script></body></html>";
    let server = MockServer::start(vec![
        Canned::page(302, "text/html", b"").header("Location", "/final"),
        Canned::page(200, "text/html", html.as_bytes()),
        Canned::page(200, "application/javascript", script.as_bytes()),
    ]);
    let env = |key: &str| match key {
        "WS_CACHE_DIR" => Some(cache.path().display().to_string()),
        _ => (key.starts_with("CLOUDFLARE_") || key == "WS_PROVIDER")
            .then(|| "unrelated malformed secret".to_owned()),
    };
    let bundled = ws::lightpanda::Bundled::new(&env);
    let url = format!("{}/start", server.url);
    let request = Request {
        url: url.clone(),
        dump: Dump::Html,
        access: Access::AllowPrivate,
        timeout: std::time::Duration::from_secs(10),
    };
    let Rendered::Html(page) = bundled.retrieve(&request).unwrap() else {
        panic!("wrong output");
    };
    assert_eq!(page.url, url);
    assert_eq!(page.final_url, format!("{}/final", server.url));
    assert!(page.body.contains("JavaScript rendered content"));
    let received = server.received();
    assert_eq!(received.len(), 3);
    for request in received {
        assert!(request.header("authorization").is_none());
        assert!(request.header("cookie").is_none());
        assert!(!request.body.contains("secret"));
    }
    let png_server = MockServer::start(vec![Canned::page(200, "text/html", HTML.as_bytes())]);
    let png_request = Request {
        url: png_server.url,
        dump: Dump::Png,
        ..request
    };
    let Rendered::Png(png) = bundled.retrieve(&png_request).unwrap() else {
        panic!("wrong output");
    };
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(png.len() > 100);
    let denied = MockServer::start(vec![Canned::page(403, "text/html", b"Denied")]);
    assert!(
        bundled
            .retrieve(&Request {
                url: denied.url,
                ..request
            })
            .unwrap_err()
            .to_string()
            .contains("HTTP 403")
    );
    let blocked = MockServer::start(vec![Canned::page(200, "text/html", HTML.as_bytes())]);
    assert!(
        bundled
            .retrieve(&Request {
                url: blocked.url.clone(),
                access: Access::PublicOnly,
                ..request
            })
            .is_err()
    );
    assert!(blocked.received().is_empty());
}

#[test]
fn switching_to_cloudflare_preserves_wire_contract_and_avoids_local_browser() {
    let server = MockServer::start(vec![Canned::json(200, &json!({"items":[]}).to_string())]);
    let fake = Fake::html(CARDS);
    let (code, _, error) = run(
        &[
            "search",
            "query",
            "--backend",
            "cloudflare",
            "--provider",
            "exa",
        ],
        &[
            ("WS_BACKEND", "lightpanda"),
            ("CLOUDFLARE_API_TOKEN", "test-token"),
            ("CLOUDFLARE_ACCOUNT_ID", "0123456789abcdef0123456789abcdef"),
            ("WS_API_BASE_URL", &server.url),
        ],
        &fake,
    );
    assert_eq!(code, 0, "{error}");
    assert!(fake.requests.borrow().is_empty());
    let request = &server.received()[0];
    let body: Value = serde_json::from_str(&request.body).unwrap();
    assert_eq!(body["provider"], "exa");
}
