# Recommended Rust delivery plan for `ws`

These are implementation recommendations, not additional Cloudflare API features. Implement against `CLOUDFLARE-WEB-SEARCH.md` and use `MCP-RUST.md` for protocol behavior.

## One binary, one shared search client

Suggested interface:

```text
ws search "query" [--provider ceramic|exa|linkup] [--limit 1..10] [--json]
ws mcp
ws --help
ws --version
```

Use `clap` for CLI parsing, `tokio` for async runtime, `reqwest` with a deliberately selected TLS backend for HTTPS, `serde`/`serde_json` for wire types, `rmcp` for MCP, and `tracing` to stderr for diagnostics. Confirm current crate releases, feature flags and minimum Rust versions when building; pin compatible versions and commit Cargo.lock for this binary. No particular crate version is mandated by Cloudflare.

Keep code cohesive: config resolution, wire types/provider enum, Cloudflare client, CLI formatting, MCP adapter. CLI and MCP must call the same validation/request/error functions. Do not add provider-specific SDK clients or deploy a Worker proxy.

## Configuration

| Environment variable | Meaning | Recommendation |
|---|---|---|
| `CLOUDFLARE_API_TOKEN` | runtime Cloudflare bearer token | required; never a positional/CLI value |
| `CLOUDFLARE_ACCOUNT_ID` | target account ID | required |
| `WS_GATEWAY_ID` | gateway | default `default` |
| `WS_PROVIDER` | provider enum | default `ceramic` |
| `WS_LIMIT` | search result maximum | default 10 |
| `WS_BYOK_ALIAS` | stored-key alias | optional; omission has documented billing semantics |
| `WS_TIMEOUT_SECS` | overall request deadline | suggested 30 seconds, configurable; not an upstream SLA |

Only the first two names are used in Cloudflare examples. All `WS_*` names are proposed application conventions. Explicit CLI flags override environment values; defaults apply last. If a config file is later added, document precedence and keep bearer secrets outside it. Reject empty required credentials, invalid enums, limits, aliases, and whitespace-only queries before network requests. Count characters rather than UTF-8 bytes; preserve the submitted query without unexplained rewriting. Empty alias configuration should be an error, not silently enable credits fallback.

Keep gateway, account, credentials, timeout and billing alias as operator configuration. Expose query/provider/limit to the model. Do not let tool calls choose an API base URL or override credentials. Test mocks can inject a loopback endpoint internally without creating a public arbitrary-host credential-forwarding option.

## Serialization and transport

- Serialize request through typed serde structures: `query`, `provider`, `limit`, optional camelCase `byokAlias`, required `options.gateway.id`. `skip_serializing_if = "Option::is_none"` should omit an absent alias rather than send null.
- Model camelCase response fields explicitly or use `rename_all = "camelCase"`. Item URL/title are strings; optional description/image/favicon/date are `Option<String>`. Latency is a JSON number, not necessarily an integer. Metadata and its members should tolerate absence. Retain unknown fields when useful for JSON output.
- Reuse one pooled HTTP client. Use HTTPS certificate verification and system-compatible roots; do not solve TLS issues by disabling verification. Disable redirects or ensure bearer tokens cannot be forwarded to another host.
- Set a connect timeout and total request deadline. Bound response body consumption (for example 2 MiB, configurable); enforce while reading, including absent/misleading Content-Length. This is local protection, not a Cloudflare response-size limit.
- Inspect status and body before decoding success. Parse the documented direct result object; handle any actual observed alternate envelope deliberately. Missing items is an unexpected response error, not a fabricated empty result. Preserve a real `items: []` as a successful no-results response.
- Preserve request ID, returned query and upstream latency independently of measured end-to-end duration. Do not invent metadata for missing fields.

## Errors and paid retries

Define a small shared error type: local configuration/validation, transport/timeout, upstream HTTP failure (status, sanitized message/code, request ID if available), unexpected response. Avoid exposing headers, bearer token, full request debug dumps, or provider secrets. Do not log queries/snippets by default. Keep error body excerpts bounded and terminal-safe.

Local recommended interpretation: 400 usually needs request/key configuration repair; 401/403 suggest authentication/scope; 429 suggests throttling; 5xx suggests transient service failure. These are generic HTTP interpretations, not documented Cloudflare search guarantees. Actual error codes/messages must be captured during live verification. Never treat an HTTP 200 body with explicit observed failure indicators as success.

Default to no automatic paid POST retries. If adding opt-in retries, bound attempts and total deadline, use jitter, honor Retry-After when present, and disclose duplicate-charge risk. Do not retry permission failures, validation failures or missing BYOK aliases. Changing providers changes price and result behavior; automatic fallback needs explicit operator policy.

Suggested CLI exits: 0 success including empty results; 2 usage/config/input error; 1 request/response failure. Human format should show title, source URL and optional snippet, with terminal escape/control characters neutralized. `--json` should emit one valid JSON object with complete results and metadata, no headings or progress lines; stderr carries errors. Document whether JSON is the raw provider object or a normalized local contract; avoid undocumented truncation. Broken stdout pipe should terminate cleanly.

## Implementation order and acceptance

1. Build configuration/types/validation and HTTP client using fixtures from extracted schema; support human and JSON CLI outputs.
2. Add stdio MCP adapter with the same shared client; use SDK protocol negotiation and clean shutdown.
3. Test meaningful boundaries: query 0/1/1024/1025 characters including Unicode, limit 0/1/10/11, alias pattern, omitted alias serialization, nested gateway shape, optional/unknown response fields, fractional latency and missing metadata.
4. Mock integration: verify POST path/trailing slash/auth header/body; empty results; 400/401/403/429/5xx; malformed JSON; HTML error body; oversized body; timeout; redirect; redacted failures. Assert CLI JSON is parseable and MCP stdout contains only protocol frames.
5. Run an actual MCP client handshake/discovery, tools listing and search invocation against mock HTTP. Verify invalid arguments, upstream tool errors and shutdown. Do not substitute process startup for a working MCP session.
6. Run `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings` where the supported feature combination permits, and `cargo test`. Build release binary and run that exact `ws --help`, `ws --version`, CLI and MCP executable.
7. With operator-provided runtime credentials, run live CLI search and live MCP tool invocation; test configured providers individually. Capture sanitized status, result/source evidence and request ID. Missing alias should fail with 400 without falling back. Avoid generating charges merely to prove unrelated cases.

Release completion requires both adapters to return live search results with source links. Without credentials, hand off mock/build proof and state that live provider access remains unverified. No CLI or server implementation is part of this documentation task.
