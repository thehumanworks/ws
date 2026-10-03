//! End-to-end tests of the CLI: in-process through `ws::cli::run` with an
//! injected environment, and against the compiled binary with `assert_cmd`.

use std::collections::HashMap;
use std::path::Path;

use crate::common::{Canned, MockServer};
use assert_cmd::Command;
use serde_json::{Value, json};

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";
const TOKEN: &str = "test-token-do-not-leak";

struct Run {
    code: u8,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], env: &[(&str, &str)]) -> Run {
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

fn base_env<'a>(server: &'a MockServer, config: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("CLOUDFLARE_ACCOUNT_ID", ACCOUNT),
        ("CLOUDFLARE_API_TOKEN", TOKEN),
        ("WS_API_BASE_URL", server.url.as_str()),
        ("WS_CONFIG", config),
    ]
}

fn results() -> String {
    json!({
        "items": [
            { "url": "https://a.example", "title": "Alpha", "description": "first" },
            { "url": "https://b.example", "title": "Beta" }
        ],
        "metadata": { "requestId": "req-42", "latencyMs": 9 }
    })
    .to_string()
}

fn sent_body(server: &MockServer) -> Value {
    let received = server.received();
    assert_eq!(received.len(), 1, "expected exactly one request");
    serde_json::from_str(&received[0].body).unwrap()
}

fn config_file(dir: &Path) -> String {
    dir.join("config.toml").to_str().unwrap().to_owned()
}

#[test]
fn search_prints_text_results_with_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let out = run(
        &["search", "cloudflare", "workers"],
        &base_env(&server, &config),
    );

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout
            .contains("1. Alpha\n   https://a.example\n   first\n")
    );
    assert!(out.stdout.contains("2. Beta\n   https://b.example\n"));
    assert!(
        out.stdout
            .contains("2 results · provider ceramic · gateway default")
    );
    assert!(out.stdout.contains("request req-42"));
    assert_eq!(out.stderr, "");
    assert_eq!(
        sent_body(&server),
        json!({
            "query": "cloudflare workers",
            "provider": "ceramic",
            "limit": 10,
            "options": { "gateway": { "id": "default" } }
        })
    );
}

#[test]
fn json_output_is_a_single_parseable_object() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let out = run(&["search", "q", "--json"], &base_env(&server, &config));

    assert_eq!(out.code, 0, "{}", out.stderr);
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value["provider"], "ceramic");
    assert_eq!(value["gateway"], "default");
    assert_eq!(value["requestId"], "req-42");
    assert_eq!(value["items"].as_array().unwrap().len(), 2);
    assert_eq!(value["metadata"]["latencyMs"], json!(9.0));
}

#[test]
fn provider_precedence_flag_env_file_default() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());

    // Save a default provider in the config file.
    let saved = run(
        &["config", "set", "provider", "linkup"],
        &[("WS_CONFIG", &config)],
    );
    assert_eq!(saved.code, 0, "{}", saved.stderr);

    // File beats the built-in default.
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    assert_eq!(run(&["search", "q"], &base_env(&server, &config)).code, 0);
    assert_eq!(sent_body(&server)["provider"], "linkup");

    // Environment beats the file.
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let mut env = base_env(&server, &config);
    env.push(("WS_PROVIDER", "exa"));
    assert_eq!(run(&["search", "q"], &env).code, 0);
    assert_eq!(sent_body(&server)["provider"], "exa");

    // Flag beats the environment.
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let mut env = base_env(&server, &config);
    env.push(("WS_PROVIDER", "exa"));
    assert_eq!(run(&["search", "q", "--provider", "ceramic"], &env).code, 0);
    assert_eq!(sent_body(&server)["provider"], "ceramic");

    // Unset restores Cloudflare's default.
    assert_eq!(
        run(&["config", "unset", "provider"], &[("WS_CONFIG", &config)]).code,
        0
    );
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    assert_eq!(run(&["search", "q"], &base_env(&server, &config)).code, 0);
    assert_eq!(sent_body(&server)["provider"], "ceramic");
}

#[test]
fn gateway_precedence_flag_env_file_default() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    assert_eq!(
        run(
            &["config", "set", "gateway", "file-gw"],
            &[("WS_CONFIG", &config)]
        )
        .code,
        0
    );

    let server = MockServer::start(vec![Canned::json(200, &results())]);
    assert_eq!(run(&["search", "q"], &base_env(&server, &config)).code, 0);
    assert_eq!(sent_body(&server)["options"]["gateway"]["id"], "file-gw");

    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let mut env = base_env(&server, &config);
    env.push(("WS_GATEWAY_ID", "env-gw"));
    assert_eq!(run(&["search", "q"], &env).code, 0);
    assert_eq!(sent_body(&server)["options"]["gateway"]["id"], "env-gw");

    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let mut env = base_env(&server, &config);
    env.push(("WS_GATEWAY_ID", "env-gw"));
    assert_eq!(run(&["search", "q", "--gateway", "flag-gw"], &env).code, 0);
    assert_eq!(sent_body(&server)["options"]["gateway"]["id"], "flag-gw");
}

#[test]
fn limit_and_alias_flags_reach_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let out = run(
        &[
            "search",
            "q",
            "-n",
            "4",
            "--byok-alias",
            "team",
            "-p",
            "exa",
            "-g",
            "g1",
        ],
        &base_env(&server, &config),
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        sent_body(&server),
        json!({
            "query": "q", "provider": "exa", "limit": 4, "byokAlias": "team",
            "options": { "gateway": { "id": "g1" } }
        })
    );
}

#[test]
fn invalid_input_exits_2_without_sending_anything() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let long_query = "x".repeat(1025);
    let cases: Vec<Vec<&str>> = vec![
        vec!["search", "   "],
        vec!["search", &long_query],
        vec!["search", "q", "--limit", "0"],
        vec!["search", "q", "--limit", "11"],
        vec!["search", "q", "--provider", "google"],
        vec!["search", "q", "--byok-alias", "bad alias"],
        vec!["search", "q", "--gateway", "bad gateway"],
        vec!["search", "q", "--timeout", "0"],
        vec!["search", "q", "--json", "--full"],
        vec!["search"],
        vec!["nonsense"],
        vec![],
    ];
    for args in cases {
        let server = MockServer::start(vec![Canned::json(200, &results())]);
        let out = run(&args, &base_env(&server, &config));
        assert_eq!(out.code, 2, "{args:?}: {}", out.stderr);
        assert_eq!(out.stdout, "", "{args:?}");
        assert!(!out.stderr.is_empty(), "{args:?}");
        assert!(
            server.received().is_empty(),
            "{args:?} must not hit the network"
        );
    }
}

#[test]
fn missing_credentials_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let out = run(
        &["search", "q"],
        &[("WS_CONFIG", &config), ("CLOUDFLARE_API_TOKEN", TOKEN)],
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("CLOUDFLARE_ACCOUNT_ID is not set"),
        "{}",
        out.stderr
    );

    let out = run(
        &["search", "q"],
        &[("WS_CONFIG", &config), ("CLOUDFLARE_ACCOUNT_ID", ACCOUNT)],
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("CLOUDFLARE_API_TOKEN is not set"),
        "{}",
        out.stderr
    );
}

#[test]
fn api_errors_exit_1_with_hint_and_no_token() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let body = r#"{"ok":false,"error":{"category":"gateway","code":"web_search_payment_required","status":402,"retryable":false,"gatewayRequestId":"gw-req"}}"#;
    let server = MockServer::start(vec![Canned::json(402, body)]);
    let out = run(&["search", "q", "--json"], &base_env(&server, &config));
    assert_eq!(out.code, 1);
    assert_eq!(out.stdout, "", "errors must not pollute stdout");
    assert!(
        out.stderr
            .contains("HTTP 402 [web_search_payment_required]"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("request id: gw-req"));
    assert!(out.stderr.contains("hint:"));
    assert!(!out.stderr.contains(TOKEN));
}

#[test]
fn base_url_override_must_be_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let out = run(
        &["search", "q"],
        &[
            ("CLOUDFLARE_ACCOUNT_ID", ACCOUNT),
            ("CLOUDFLARE_API_TOKEN", TOKEN),
            ("WS_API_BASE_URL", "https://evil.example"),
            ("WS_CONFIG", &config),
        ],
    );
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("local testing only"), "{}", out.stderr);
}

#[test]
fn providers_lists_all_three_and_marks_the_selected_one() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let out = run(&["providers"], &[("WS_CONFIG", &config)]);
    assert_eq!(out.code, 0);
    for name in ["* ceramic", "  exa", "  linkup"] {
        assert!(out.stdout.contains(name), "{}", out.stdout);
    }

    let out = run(
        &["providers", "--json"],
        &[("WS_CONFIG", &config), ("WS_PROVIDER", "linkup")],
    );
    let value: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(value[2]["name"], "linkup");
    assert_eq!(value[2]["selected"], true);
    assert_eq!(value[0]["selected"], false);
}

#[test]
fn config_show_reports_sources_and_never_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    assert_eq!(
        run(
            &["config", "set", "gateway", "saved-gw"],
            &[("WS_CONFIG", &config)]
        )
        .code,
        0
    );
    let out = run(
        &["config", "show"],
        &[
            ("WS_CONFIG", &config),
            ("WS_PROVIDER", "exa"),
            ("CLOUDFLARE_API_TOKEN", TOKEN),
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("provider     exa (environment)"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("gateway      saved-gw (config file)"));
    assert!(out.stdout.contains("account id   not set"));
    assert!(out.stdout.contains("api token    set"));
    assert!(!out.stdout.contains(TOKEN));

    let path = run(&["config", "path"], &[("WS_CONFIG", &config)]);
    assert_eq!(path.stdout.trim(), config);
}

#[test]
fn config_set_rejects_invalid_values_and_bad_files() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    assert_eq!(
        run(
            &["config", "set", "provider", "google"],
            &[("WS_CONFIG", &config)]
        )
        .code,
        2
    );
    assert_eq!(
        run(
            &["config", "set", "gateway", "a b"],
            &[("WS_CONFIG", &config)]
        )
        .code,
        2
    );
    assert!(
        !Path::new(&config).exists(),
        "nothing may be written for invalid values"
    );

    std::fs::write(&config, "provider = \"google\"\n").unwrap();
    let out = run(&["providers"], &[("WS_CONFIG", &config)]);
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("config file"), "{}", out.stderr);
}

#[test]
fn help_and_version_exit_0_on_stdout() {
    let help = run(&["--help"], &[]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.contains("WS_GATEWAY_ID"));
    assert!(
        help.stdout
            .contains("Precedence: flag > environment > config file > default.")
    );
    assert_eq!(help.stderr, "");

    let version = run(&["--version"], &[]);
    assert_eq!(version.code, 0);
    assert_eq!(
        version.stdout.trim(),
        concat!("ws ", env!("CARGO_PKG_VERSION"))
    );
}

// --- The compiled binary -----------------------------------------------------

fn binary(config: &str) -> Command {
    let mut command = Command::cargo_bin("ws").unwrap();
    // Start from a clean environment so the developer's real credentials and
    // settings can never leak into (or be billed by) the test run.
    let _: &mut Command = command
        .env_clear()
        .env("WS_BACKEND", "cloudflare")
        .env("WS_CONFIG", config);
    // Windows cannot open sockets without SystemRoot.
    if let Some(root) = std::env::var_os("SystemRoot") {
        let _: &mut Command = command.env("SystemRoot", root);
    }
    command
}

#[test]
fn binary_runs_a_search_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let server = MockServer::start(vec![Canned::json(200, &results())]);
    let output = binary(&config)
        .env("CLOUDFLARE_ACCOUNT_ID", ACCOUNT)
        .env("CLOUDFLARE_API_TOKEN", TOKEN)
        .env("WS_API_BASE_URL", &server.url)
        .env("WS_GATEWAY_ID", "bin-gw")
        .env("WS_PROVIDER", "exa")
        .args(["search", "hello world", "--json", "-n", "2"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["provider"], "exa");
    assert_eq!(value["gateway"], "bin-gw");

    let received = server.received();
    assert_eq!(
        received[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    let body: Value = serde_json::from_str(&received[0].body).unwrap();
    assert_eq!(
        body,
        json!({
            "query": "hello world", "provider": "exa", "limit": 2,
            "options": { "gateway": { "id": "bin-gw" } }
        })
    );
}

#[test]
fn binary_exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_file(dir.path());
    let _ = binary(&config).arg("--version").assert().success();
    let _ = binary(&config).arg("providers").assert().success();
    let _ = binary(&config).args(["search", "q"]).assert().code(2);
    let _ = binary(&config)
        .args(["search", "q", "--limit", "99"])
        .assert()
        .code(2);

    let server = MockServer::start(vec![Canned::json(500, "boom")]);
    let _ = binary(&config)
        .env("CLOUDFLARE_ACCOUNT_ID", ACCOUNT)
        .env("CLOUDFLARE_API_TOKEN", TOKEN)
        .env("WS_API_BASE_URL", &server.url)
        .args(["search", "q"])
        .assert()
        .code(1);
}
