# ws

Web search from your terminal, through Cloudflare's
[AI Gateway Web Search API](https://developers.cloudflare.com/web-search/).
One static binary for macOS, Linux and Windows.

```console
$ ws search "What is Cloudflare Workers?" --provider exa --limit 2
1. Cloudflare Workers - Global Serverless Functions Platform
   https://www.cloudflare.com/products/workers/
   Cloudflare Workers - Global Serverless Functions Platform ... ### Cloudflare Workers are fast, elastic, and serverless functions that scale automatically from zero to millions of requests. …

2. Overview · Cloudflare Workers docs
   https://developers.cloudflare.com/workers/
   A serverless platform for building, deploying, and scaling apps across Cloudflare's global network ↗︎ with a single command — no infrastructure to manage, no complex configuration …

2 results · provider exa · gateway iris · 4321 ms · request 4c256eee-e7bf-47c9-93e3-9fcb9c50b007
```

## Status

The API is in open beta (announced 2026-10-02). `ws` has been run against the
live endpoint on a funded gateway: searches with `ceramic` and `exa` return
results, and the response is the documented bare `{items, metadata}` object.
`linkup` answered `402 web_search_payment_required` on the same gateway, so a
successful Linkup search has not been observed. See
[Observed live](#observed-live-2026-10-02).

## Install

Download a binary for your platform from the
[releases page](https://github.com/thehumanworks/ws/releases), or build from source:

```sh
mise install                      # toolchain (Rust and friends), pinned in mise.toml
cargo build --release --locked    # -> target/release/ws
```

Cross-compile every platform from one machine with `mise run cross`:

| Target | Binary |
|---|---|
| macOS (host) | `target/release/ws` |
| Linux x86_64, static | `target/x86_64-unknown-linux-musl/release/ws` |
| Linux arm64, static | `target/aarch64-unknown-linux-musl/release/ws` |
| Windows x86_64 | `target/x86_64-pc-windows-gnu/release/ws.exe` |

TLS is rustls with bundled roots, so there is no OpenSSL to install anywhere.

## Set up Cloudflare

1. **API token**: create a custom token with **Account > Workers AI > Read** and
   **Account > AI Gateway > Read**. Export it as `CLOUDFLARE_API_TOKEN`.
2. **Account ID**: export it as `CLOUDFLARE_ACCOUNT_ID` (32 hex characters).
3. **Gateway**: every account has a gateway called `default`. To use another, pass
   `--gateway` or set `WS_GATEWAY_ID`.
4. **Funding**: load AI Gateway credits (AI Gateway > Manage > Top-up credits), or
   store your own provider key on the gateway and pass `--byok-alias`.

```sh
export CLOUDFLARE_API_TOKEN=...      # or let fnox / 1Password / your shell provide these
export CLOUDFLARE_ACCOUNT_ID=...
ws config show                       # confirms what ws sees, without printing secrets
```

The token is read only from the environment. It is never accepted as a flag,
written to the config file, or printed.

## Use

```sh
ws search rust async runtimes               # words are joined into one query
ws search "rust async runtimes" -n 5        # 1-10 results (default 10)
ws search "rust async runtimes" -p exa      # choose a provider for this search
ws search "rust async runtimes" --json      # one JSON object, complete and untruncated
ws search "rust async runtimes" --full      # whole snippets instead of 300 characters
ws providers                                # the three providers; * marks the active one
ws config set provider linkup               # save a default provider
ws config set gateway team-gateway          # save a default gateway
ws config unset provider                    # back to Cloudflare's default
ws config show                              # what is in effect, and where it came from
```

### Providers

| Name | USD per 1,000 searches | Behaviour |
|---|---:|---|
| `ceramic` (Cloudflare's default) | 0.25 | Independent index of 40B+ pages; snippets up to 8,000 characters |
| `exa` | 7.00 | `auto` search type; highlights returned as the snippet |
| `linkup` | 5.00 | `fast` search depth; raw results |

Prices are a snapshot of Cloudflare's documentation on 2026-10-02.

### Configuration

Each setting is taken from the first place that provides it:

**flag > environment variable > config file > default**

| Setting | Flag | Environment | Config file key | Default |
|---|---|---|---|---|
| Provider | `-p`, `--provider` | `WS_PROVIDER` | `provider` | `ceramic` |
| Gateway ID | `-g`, `--gateway` | `WS_GATEWAY_ID` | `gateway_id` | `default` |
| Result limit | `-n`, `--limit` | `WS_LIMIT` | | `10` |
| BYOK alias | `--byok-alias` | `WS_BYOK_ALIAS` | | none (credits) |
| Timeout (s) | `--timeout` | `WS_TIMEOUT_SECS` | | `30` |
| Account ID | `--account-id` | `CLOUDFLARE_ACCOUNT_ID` | | required |
| API token | | `CLOUDFLARE_API_TOKEN` | | required |
| Config path | | `WS_CONFIG` | | see below |

So a saved default provider can be overridden for a shell session with
`WS_PROVIDER`, and for one command with `--provider`.

The config file is `$XDG_CONFIG_HOME/ws/config.toml` (or `~/.config/ws/config.toml`),
and `%APPDATA%\ws\config.toml` on Windows:

```toml
provider = "exa"
gateway_id = "team-gateway"
```

An environment variable that is set but empty is an error, not a silent fall-through.

### Output and exit status

Text output is numbered results with URL and snippet; control characters from
the web are neutralised so a page title cannot drive your terminal. `--json`
prints exactly one object and nothing else on stdout:

```json
{
  "provider": "linkup",
  "gateway": "default",
  "requestId": "…",
  "items": [{ "url": "…", "title": "…", "description": "…" }],
  "metadata": { "query": "…", "requestId": "…", "latencyMs": 12.5 }
}
```

`items` and `metadata` are Cloudflare's, including any fields added later.

| Exit | Meaning |
|---|---|
| 0 | Success (including zero results) |
| 1 | The request failed: network, timeout, API error, unexpected response |
| 2 | Usage, configuration or input error. Nothing was sent, nothing was billed |

Errors go to stderr with Cloudflare's code, the request ID and a hint:

```console
$ ws search anything
ws: Cloudflare API error: HTTP 402 [web_search_payment_required]
  request id: 8c91cf7e-e585-4a7f-ae93-a2c8a24f4df9
  hint: the gateway has no funding for this provider: load AI Gateway credits ...
```

`ws` never retries: every search is billed and the API has no idempotency key.
It does not follow redirects, and caps responses at 2 MiB.

## Demo

```sh
demo/demo.sh --mock    # full tour against a local mock: no credentials, no billing
demo/demo.sh           # the same tour against Cloudflare (billed)
```

[`demo/transcript.txt`](demo/transcript.txt) is the recorded output of the mock
tour: providers, searching, JSON, saving a default provider, overriding it by
environment and by flag, choosing a gateway, and the error cases.

## Verification

`mise run verify` runs all of it; the `hk` pre-commit hook and CI run the same.

| Layer | What it checks | Run |
|---|---|---|
| Unit and integration tests | 83 tests: validation boundaries (Unicode included), wire format, precedence, every error shape, redirects, timeouts, oversize bodies, the compiled binary | `mise run test` |
| Lean 4 proofs | Precedence, provider parsing and request bounds, proved for all inputs | `mise run proof` |
| Model conformance | The Rust code is replayed against 105 decisions generated from the Lean model | part of `mise run test` |
| Clippy, strict | `pedantic` + `nursery` + no `unwrap`/`panic`/indexing/`allow`, warnings denied | `mise run lint` |
| ast-grep rules | 8 project rules with their own tests | `mise run rules` |
| Cross builds | Linux x86_64/arm64 and Windows binaries link | `mise run cross` |

### What Lean proves

[`lean/WsSpec`](lean/WsSpec) models the decisions that cost money or surprise
users when wrong, with no `sorry` and only Lean's standard axioms:

- **Precedence** (`Precedence.lean`): a flag always wins; the environment wins
  without a flag; the file wins without either; the default applies only when all
  three are silent; the reported source is the layer the value really came from;
  with nothing configured the provider is Ceramic; every provider is selectable
  from every layer.
- **Providers** (`Provider.lean`): there are exactly three; `parse` and `name`
  are inverse, so a name maps to one provider and unknown names are rejected.
- **Requests** (`Validation.lean`): any request that validation lets through has
  a 1-1,024 character query, a limit of 1-10 and a non-empty gateway; valid input
  is never refused; validation never alters the input.

The proofs are about the Lean model, not the Rust source. The link between them
is `lean/vectors.txt`: the model prints its decision for all 64 provider-layer
combinations, 8 gateway combinations, limits 0-20, the query-length boundaries
and provider names, and `tests/it/lean_vectors.rs` requires the Rust code to
agree on every line. `mise run proof` fails if that file is stale.

### ast-grep rules

| Rule | Enforces |
|---|---|
| `no-env-access` | Only `main.rs` reads the environment; everything else is injected |
| `no-direct-stdio` | Output goes through injected writers, never `println!` |
| `no-panicking-calls` | No `unwrap`/`expect`/`panic!` outside test modules |
| `no-process-exit` | Exit status is returned, not forced |
| `no-lint-allow` | No `#[allow]`; use `#[expect(.., reason)]` |
| `no-byte-length-on-query` | Query length is counted in characters |
| `no-retry-loop-around-search` | A billed request is never retried in a loop |
| `no-token-in-output` | The token is never formatted into output |

### Observed live (2026-10-02)

| Case | Result |
|---|---|
| Search with `ceramic`, funded gateway | `200`, results, bare `{items, metadata}` |
| Search with `exa`, funded gateway | `200`, results |
| Search with `linkup`, same gateway | `402` `web_search_payment_required` |
| Query with no matches | `200`, `"items": []`, metadata present |
| Any provider, unfunded gateway | `402` `web_search_payment_required` |
| BYOK alias that is not stored | `400` `web_search_byok_not_configured` |
| Gateway that does not exist | `400` code `2001` |
| Limit above 10 (sent with curl, bypassing `ws`) | `400` code `7000`, field detail on `limit` |
| Invalid token | `400` code `9106` |
| Query of 1,024 four-byte emoji (4,096 bytes) | Accepted by validation (`402`, not `400`) |
| Query of 1,025 characters, ASCII or emoji | `400` `invalid_web_search_input` |
| Whitespace-only query | `400` code `7000`, "at least 1 character" |

The last three confirm that Cloudflare counts the 1,024 limit in characters
(Unicode scalar values), not bytes or UTF-16 units, as `ws` and the Lean model do.
The validation probes were not billed; the successful searches were (six in total).

These bodies are test fixtures in `src/client.rs`.

## Not included

- No MCP server. `docs/MCP-RUST.md` plans one; this project is the CLI only.
- No retries, pagination, domain or date filters: the API documents none.

## Contributing

See [AGENTS.md](AGENTS.md) for the development loop, the rules, and how to
change validated behaviour.
