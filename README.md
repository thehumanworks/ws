# ws

Web search and JavaScript page reading from one executable. Select
`--backend lightpanda` for the bundled local browser, or `--backend cloudflare`
for [AI Gateway Web Search](https://developers.cloudflare.com/web-search/).
`ws fetch` then reads any page a search finds: its main content, as Markdown,
HTML, JSON or a text-only PNG. macOS and GNU Linux releases include compressed
Lightpanda 1.0.0; there is no separate browser installation. Windows uses Cloudflare.

```sh
ws search "rust async runtimes" --backend lightpanda --limit 5
ws search "rust async runtimes" --backend cloudflare --provider exa
ws fetch https://example.com/app --backend lightpanda
ws fetch https://example.com/app --backend lightpanda --format png > page.png
ws fetch https://example.com/app --backend cloudflare --render
ws fetch https://example.com/article --backend cloudflare  # direct HTTP, no browser
ws config set backend lightpanda
```

Lightpanda is the default on the four bundled macOS/Linux targets; Cloudflare
is the default elsewhere. Selection is flag > `WS_BACKEND` > saved `backend` >
platform default. It is independent of Cloudflare's `ceramic`, `exa` and `linkup`
provider preferences. Lightpanda search loads one ordinary Brave search URL,
extracts organic results in rank order, excludes ads and deduplicates URLs.
Access denials, bot challenges and unrecognized layouts produce errors; there
is no retry, challenge bypass or automatic backend fallback.

Choose a backend for the current shell session without repeating the flag:

```sh
export WS_BACKEND=lightpanda  # or cloudflare
ws search "rust async runtimes"
ws fetch https://example.com
unset WS_BACKEND             # restore the saved or platform default
```

Lightpanda fetch always runs JavaScript. `--render` is accepted as a redundant
request on that backend; it triggers metered Browser Run only on Cloudflare.
`--raw` preserves the complete rendered DOM, while normal Markdown/HTML/JSON
uses the existing content selection. PNG writes validated binary bytes and
renders the whole page's **text**, including with `--raw`; it is not a graphical
browser screenshot. Cloudflare plus PNG is a usage error and never starts billing.

Cloudflare credentials are required only for its search and explicit render.
Lightpanda works with absent or malformed unrelated Cloudflare settings; explicit
`--provider`, `--gateway`, `--account-id` and `--byok-alias` require Cloudflare.

```console
$ ws search "What is Cloudflare Workers?" --backend cloudflare --provider exa --limit 2
1. Cloudflare Workers - Global Serverless Functions Platform
   https://www.cloudflare.com/products/workers/
   Cloudflare Workers - Global Serverless Functions Platform ... ### Cloudflare Workers are fast, elastic, and serverless functions that scale automatically from zero to millions of requests. …

2. Overview · Cloudflare Workers docs
   https://developers.cloudflare.com/workers/
   A serverless platform for building, deploying, and scaling apps across Cloudflare's global network ↗︎ with a single command — no infrastructure to manage, no complex configuration …

2 results · provider exa · gateway iris · 4321 ms · request 4c256eee-e7bf-47c9-93e3-9fcb9c50b007
```

```console
$ ws fetch https://example.com
This domain is for use in documentation examples without needing permission. This is not a service; avoid relying on it for testing and monitoring purposes.
```

## Status

The Cloudflare API is in open beta (announced 2026-10-02). `ws` has been run against the
live endpoint on a funded gateway: searches with `ceramic` and `exa` return
results, and the response is the documented bare `{items, metadata}` object.
`linkup` answered `402 web_search_payment_required` on the same gateway, so a
successful Linkup search has not been observed. See
[Observed live](#observed-live-2026-10-02).

`ws fetch --backend cloudflare` requests the page directly, so it needs no token,
gateway or credits. Only that backend's `--render` uses Cloudflare (Browser Run); it has been run
against the live endpoint too.

## Install

Download a binary for your platform from the
[releases page](https://github.com/thehumanworks/ws/releases), or build from source:

```sh
mise install                      # toolchain (Rust and friends), pinned in mise.toml
cargo build --release --locked    # -> target/release/ws
```

Cross-compile Linux and Windows with `mise run cross`:

| Target | Binary |
|---|---|
| macOS (host) | `target/release/ws` |
| GNU Linux x86_64 | `target/x86_64-unknown-linux-gnu/release/ws` |
| GNU Linux arm64 | `target/aarch64-unknown-linux-gnu/release/ws` |
| Windows x86_64 | `target/x86_64-pc-windows-gnu/release/ws.exe` |

The Rust HTTP clients use rustls with bundled roots. Lightpanda uses the system
trust store. Its pinned Linux browser needs **glibc 2.35+ on x86_64** and
**glibc 2.38+ on ARM64**, verified from ELF symbol requirements with
`scripts/inspect-lightpanda-elf.py`. Alpine/musl is not supported by this browser.
Its Mach-O minimum OS metadata is **macOS 14.8.7 on Intel** and **14.8.9 on
Apple Silicon**. These are the browser's requirements; source builds without a
supported bundle default to Cloudflare and explicit Lightpanda fails helpfully.

`build.rs` selects assets from Cargo's `TARGET`, verifies the immutable size and
SHA256 pins, compresses them and embeds the result. The first source build
downloads its browser from GitHub using `curl`; further builds reuse the
verified cache in `target/lightpanda-assets`. Set `WS_LIGHTPANDA_ASSET_DIR` to a
directory containing the pinned upstream executables (names in
`assets/lightpanda-pins.txt`) or previously generated hash-named `.gz` files.
`WS_LIGHTPANDA_OFFLINE=1 cargo build --release --locked` forbids provisioning
downloads and reports missing assets. Cargo's own dependencies must also be cached.

First runtime use extracts the browser into a private version/target/hash cache:
`WS_CACHE_DIR/ws`, else `XDG_CACHE_HOME/ws`, else macOS `~/Library/Caches/ws` or
Linux `~/.cache/ws`. Extraction is verified and atomically published; every
reuse verifies the executable, and symlink/corrupt/writable entries are refused.
The extracted browser is approximately 86–184 MiB depending on architecture.
Release packages retain pinned source and licensing notices; see
[THIRD_PARTY.md](THIRD_PARTY.md) for AGPL source/distribution considerations.

## Set up Cloudflare

1. **API token**: create a custom token with **Account > Workers AI > Read** and
   **Account > AI Gateway > Read**. Export it as `CLOUDFLARE_API_TOKEN`. Add
   **Account > Browser Rendering > Edit** if you want `ws fetch --render`.
2. **Account ID**: export it as `CLOUDFLARE_ACCOUNT_ID` (32 hex characters).
3. **Gateway**: every account has a gateway called `default`. To use another, pass
   `--gateway` or set `WS_GATEWAY_ID`.
4. **Funding**: load AI Gateway credits (AI Gateway > Manage > Top-up credits), or
   store your own provider key on the gateway and pass `--byok-alias`.

```sh
export CLOUDFLARE_API_TOKEN=...      # or let fnox / 1Password / your shell provide these
export CLOUDFLARE_ACCOUNT_ID=...
export WS_BACKEND=cloudflare
ws config show                       # confirms what ws sees, without printing secrets
```

The token is read only from the environment. It is never accepted as a flag,
written to the config file, or printed.

## Use

```sh
ws search rust async runtimes               # words are joined into one query
ws search "rust async runtimes" -n 5        # 1-10 results (default 10)
ws search "rust async runtimes" --backend cloudflare -p exa # Cloudflare provider
ws search "rust async runtimes" --json      # one JSON object, complete and untruncated
ws search "rust async runtimes" --full      # whole snippets instead of 300 characters
ws fetch https://example.com/article        # a page's main content as Markdown
ws fetch https://example.com/article -f json  # one JSON object: URLs, status, title, Markdown
ws fetch https://example.com/article -f html  # the main content as HTML
ws fetch https://example.com/article --raw  # the whole page, nothing removed
ws fetch https://example.com/app --backend cloudflare --render # metered Browser Run
ws providers                                # providers for the selected backend
ws config set backend cloudflare            # select the other runtime
ws config set provider linkup               # save a Cloudflare provider preference
ws config set gateway team-gateway          # save a default gateway
ws config unset provider                    # back to Cloudflare's default
ws config show                              # what is in effect, and where it came from
```

### Cloudflare providers

| Name | USD per 1,000 searches | Behaviour |
|---|---:|---|
| `ceramic` (Cloudflare's default) | 0.25 | Independent index of 40B+ pages; snippets up to 8,000 characters |
| `exa` | 7.00 | `auto` search type; highlights returned as the snippet |
| `linkup` | 5.00 | `fast` search depth; raw results |

Prices are a snapshot of Cloudflare's documentation on 2026-10-02.

### Fetching pages

`ws fetch <URL>` is the second half of "search, then read". Lightpanda renders
locally; Cloudflare without `--render` makes one direct `GET` (following redirects).
Both print the selected main content through the same pure conversion pipeline.

| `--format` | Output |
|---|---|
| `markdown` (default) | The main content converted to Markdown |
| `html` | The main content as HTML (with `--raw`: the body exactly as served) |
| `json` | One object with the request context and the Markdown (below) |
| `png` | Lightpanda only: binary, whole-page text rendering |

```json
{
  "url": "https://example.com",
  "finalUrl": "https://example.com/",
  "status": 200,
  "contentType": "text/html; charset=utf-8",
  "title": "Example Domain",
  "extracted": true,
  "rendered": false,
  "markdown": "This domain is for use in documentation examples …"
}
```

`contentType` and `title` are omitted when the page has none. `extracted` says
whether only the main content was kept; `rendered` whether a browser was used.

#### What is kept

By default `ws fetch` keeps what an agent came for and drops the rest. The
rules are fixed lists, not scores, so the same page always gives the same
answer:

1. **Removed everywhere:**
   - non-content: `<head>`, `script`, `style`, `noscript`, `template`, `iframe`,
     `svg`, `canvas`, comments, inline `data:` images;
   - page furniture and controls: `nav`, `footer`, `aside`, `dialog`, `menu`,
     `button`, `input`, `select`, `textarea`, `datalist`, and a `<header>` that
     is not inside `<main>` or `<article>`;
   - ARIA roles `navigation`, `banner`, `contentinfo`, `complementary`, `search`,
     `dialog`, `alertdialog`, `menu`, `menubar`, `toolbar`, `tablist`;
   - hidden elements: `hidden`, `aria-hidden="true"`, inline `display:none` or
     `visibility:hidden`;
   - elements whose `class` or `id` contains a boilerplate word (`nav`, `navbar`,
     `navigation`, `menu`, `dropdown`, `sidebar`, `breadcrumb(s)`, `footer`,
     `cookie(s)`, `consent`, `advert`, `advertisement`, `ads`, `promo`,
     `newsletter`, `subscribe`, `share`, `sharing`, `social`, `popup`, `modal`,
     `toolbar`), unless the element holds half or more of the page's text, in
     which case the name is a layout wrapper such as `has-sidebar`.
2. **Then narrowed** to the first `<main>` (or `role="main"`) that has text;
   failing that the only `<article>`; failing that the `<body>`.

`--raw` switches both steps off: the whole page is converted (only `<head>`,
scripts, styles and the other non-content elements are left out of the
Markdown), and `--format html --raw` is the response body as served.

Removed elements are never brought back, so a page that is all navigation comes
out empty; `ws` then says so on stderr and suggests `--raw` or `--render`.
Forms are kept (some sites wrap the whole page in one); only their controls go.
Pages already served as Markdown, plain text, JSON or XML are printed as
served: there is nothing to extract from.

Measured on 2026-10-03 (Markdown bytes, `--raw` then default):

| Page | Whole page | Main content |
|---|---:|---:|
| MDN, the `<main>` element | 36,081 | 7,834 |
| Wikipedia, "Rust (programming language)" | 287,305 | 203,910 |
| docs.rs, `ureq` | 28,825 | 23,466 |
| BBC News front page | 22,155 | 16,907 |
| Hacker News front page (tables, no landmarks) | 10,778 | 10,778 |

#### JavaScript pages: `--render`

A plain Cloudflare-backend fetch does not run JavaScript, so a page built in the
browser comes back nearly empty. On that backend, `--render` asks
[Cloudflare Browser Run](https://developers.cloudflare.com/browser-run/quick-actions/content-endpoint/)
to load the page in a real browser (waiting for the network to settle) and
return the resulting HTML, which then goes through the same extraction:

```console
$ ws fetch https://quotes.toscrape.com/js/ --backend cloudflare
# [Quotes to Scrape](/)

[Login](/login)
$ ws fetch https://quotes.toscrape.com/js/ --backend cloudflare --render
# [Quotes to Scrape](/)

[Login](/login)

“The world as we have created it is a process of our thinking. It cannot be changed without changing our thinking.”by Albert Einstein
…
```

On the Cloudflare backend, `--render` is an API call: it needs
`CLOUDFLARE_API_TOKEN` (with **Browser Rendering > Edit**) and
`CLOUDFLARE_ACCOUNT_ID`, it uses browser time that Cloudflare meters, and the
page is loaded from Cloudflare's network, not yours. It is therefore never
automatic: Cloudflare renders only when asked, makes exactly one request and never
retries. `finalUrl` is the requested URL (Browser Run does not report
redirects).

#### Safety and limits

- **No credentials on a plain fetch.** Cloudflare is not involved and the API
  token is never sent: `ws fetch` works with `CLOUDFLARE_*` unset. The only
  backend settings and shared timeout/limit preferences remain independent of credentials.
- **Public addresses only.** A host that resolves to loopback, a private or
  link-local network (including cloud metadata at `169.254.169.254`),
  carrier-grade NAT, multicast or the IPv6 equivalents is refused with exit 1
  before any connection is made. The check runs after DNS resolution and again
  on every redirect, so neither a hostname nor a redirect can reach inside your
  network. Lightpanda also applies the filter to script/XHR/fetch/iframe/WebSocket
  sockets, with supplemental compatible/NAT64 CIDRs matching ws's address policy.
  `--allow-private` turns it off for a trusted local page.
  (If `HTTP_PROXY`/`HTTPS_PROXY` is set, the proxy resolves the target and this
  check cannot see it.)
- **Direct HTTP Markdown where the site offers it.** A Cloudflare-backend plain fetch says
  `Accept: text/markdown` first, so sites that serve Markdown to agents are
  passed through unconverted.
- **Redirects are followed** (direct HTTP: up to 10); `finalUrl` is the page that answered.
- **Bounded.** gzip is accepted, and pages over 5 MiB *after decompression* are
  refused, as are non-text types such as PDFs and images (decided from
  `Content-Type`, before the body is downloaded). Declared charsets are decoded
  to UTF-8.
- **Terminal-safe.** Control characters other than newline and tab are
  neutralised in Markdown and HTML output; JSON escapes them.
- **Local browser limits.** The child inherits no user environment or credentials;
  telemetry/core dumps are disabled. Per-response/rendered-body limits are 5 MiB,
  captured stdout/stderr limits are 64 MiB/64 KiB, V8 heap is capped at 256 MiB,
  and the parent enforces `--timeout` across navigation and dumping. PNG has the
  same 5 MiB encoded-file limit and a 128 MiB decoded-pixel buffer limit.
- A non-2xx answer, a blocked host, a timeout or an unreadable page exits 1; a
  URL that is not absolute `http://` or `https://` exits 2 and sends nothing.
- Not done: robots.txt, crawling, pagination, truncation.

The pieces are separate on purpose: `src/fetch.rs`, `src/render.rs` and `src/lightpanda.rs` each
retrieve a `Page`, `src/extract.rs` selects the content, `src/markdown.rs`
converts it, `src/output.rs` prints it. Structured scraping is not implemented;
it would be another consumer of `Page` or `extract::Document`.

### Configuration

Each setting is taken from the first place that provides it:

**flag > environment variable > config file > default**

| Setting | Flag | Environment | Config file key | Default |
|---|---|---|---|---|
| Backend | `--backend` | `WS_BACKEND` | `backend` | Lightpanda on bundled targets, Cloudflare elsewhere |
| Provider | `-p`, `--provider` | `WS_PROVIDER` | `provider` | `ceramic` |
| Gateway ID | `-g`, `--gateway` | `WS_GATEWAY_ID` | `gateway_id` | `default` |
| Result limit | `-n`, `--limit` | `WS_LIMIT` | | `10` |
| BYOK alias | `--byok-alias` | `WS_BYOK_ALIAS` | | none (credits) |
| Timeout (s) | `--timeout` | `WS_TIMEOUT_SECS` | | `30` |
| Account ID | `--account-id` (search) | `CLOUDFLARE_ACCOUNT_ID` | | Cloudflare search/render only |
| API token | | `CLOUDFLARE_API_TOKEN` | | Cloudflare search/render only |
| Config path | | `WS_CONFIG` | | see below |

So a saved default provider can be overridden for a shell session with
`WS_PROVIDER`, and for one command with `--provider`.

The config file is `$XDG_CONFIG_HOME/ws/config.toml` (or `~/.config/ws/config.toml`),
and `%APPDATA%\ws\config.toml` on Windows:

```toml
backend = "cloudflare"
provider = "exa"
gateway_id = "team-gateway"
```

An active setting that is set but empty is an error. Unrelated Cloudflare settings
are left unvalidated while using Lightpanda; TOML syntax, unknown keys and wrong
field types still fail. Invalid Cloudflare names/gateway strings are ignored
locally, while the config file remains a typed document.

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

This is the preserved Cloudflare JSON contract. Local JSON instead has
`backend: "lightpanda"`, `provider: "brave"`, `items` and `elapsedMs`; it has
no gateway, billed metadata or invented request ID.

| Exit | Meaning |
|---|---|
| 0 | Success (including zero results) |
| 1 | The request failed: network, timeout, API error, unexpected response |
| 2 | Usage, configuration or input error. Nothing was sent, nothing was billed |

Errors go to stderr with Cloudflare's code, the request ID and a hint:

```console
$ ws search anything --backend cloudflare
ws: Cloudflare API error: HTTP 402 [web_search_payment_required]
  request id: 8c91cf7e-e585-4a7f-ae93-a2c8a24f4df9
  hint: the gateway has no funding for this provider: load AI Gateway credits ...
```

`ws` never retries. Cloudflare searches are billed and have no idempotency key.
A Cloudflare search does not follow redirects (the token must not travel), and caps
responses at 2 MiB after decompression. `ws fetch` carries no token, so it does
follow redirects; see [Fetching pages](#fetching-pages).

## Demo

```sh
demo/demo.sh --mock    # full tour against a local mock: no credentials, no billing
demo/demo.sh           # the same tour against Cloudflare (searches are billed; one --render uses Browser Run time)
```

[`demo/transcript.txt`](demo/transcript.txt) is the recorded output of the mock
tour: providers, searching, JSON, fetching a page (main content, `--raw`, each format, a blocked host, `--render`), saving a default provider, overriding it by
environment and by flag, choosing a gateway, and the error cases.

## Verification

`mise run verify` runs all of it; the `hk` pre-commit hook and CI run the same.

| Layer | What it checks | Run |
|---|---|---|
| Unit and integration tests | 136 tests: validation boundaries (Unicode included), wire format, precedence, every error shape, redirects, timeouts, oversize and gzip-bomb bodies, content extraction, page fetching in every format, private-address blocking, `--render`, the compiled binary | `mise run test` |
| Lean 4 proofs | Precedence, provider parsing, search request bounds, and the fetch rules (URLs, public addresses, media types), proved for all inputs | `mise run proof` |
| Model conformance | The Rust code is replayed against 187 decisions generated from the Lean model | part of `mise run test` |
| Clippy, strict | `pedantic` + `nursery` + no `unwrap`/`panic`/indexing/`allow`, warnings denied | `mise run lint` |
| ast-grep rules | 9 project rules with their own tests | `mise run rules` |
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
- **Fetching** (`Fetch.lean`): an accepted URL is at most 2,048 characters, has
  no space or control character (so it cannot smuggle a header line), and has an
  `http`/`https` scheme followed by a host; loopback, the RFC 1918 ranges,
  link-local, `0.0.0.0/8` and multicast upwards are never public; only
  `text/html` and `application/xhtml+xml` are treated as HTML.

The proofs are about the Lean model, not the Rust source. The link between them
is `lean/vectors.txt`: the model prints its decision for all 64 provider-layer
combinations, 8 gateway combinations, limits 0-20, the query-length boundaries
and provider names, plus 23 URLs, 6 URL lengths, 35 addresses and 18 media
types for fetch, and `tests/it/lean_vectors.rs` requires the Rust code to
agree on every line. `mise run proof` fails if that file is stale.

The fetch model is narrower than the code in three stated ways: Rust also
rejects Unicode whitespace in URLs, normalises a `Content-Type` before
classifying it, and applies the address rule to IPv6 (tested in Rust only). The
content-extraction rules are not modelled; they are fixed lists with Rust tests.

### ast-grep rules

| Rule | Enforces |
|---|---|
| `no-env-access` | Only `main.rs` reads the environment; everything else is injected |
| `no-direct-stdio` | Output goes through injected writers, never `println!` |
| `no-panicking-calls` | No `unwrap`/`expect`/`panic!` outside test modules |
| `no-process-exit` | Exit status is returned, not forced |
| `no-lint-allow` | No `#[allow]`; use `#[expect(.., reason)]` |
| `no-byte-length-on-query` | Query length is counted in characters |
| `no-retry-loop-around-search` | A billed request (search or render) is never retried in a loop |
| `no-token-in-output` | The token is never formatted into output |
| `no-credentials-in-fetch` | The page-fetching modules never touch credentials or auth headers |

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

On 2026-10-03, Browser Run's `/browser-run/content` answered `200` with
`{"success": true, "result": "<html>…"}` for `example.com` and for a
JavaScript-built page (`quotes.toscrape.com/js/`), whose quotes appear only when
rendered. That success shape is a fixture in `src/render.rs`. No Browser Run
error has been observed live; the error tests use Cloudflare's standard
`{"success": false, "errors": [...]}` envelope.

## Not included

- No MCP server. `docs/MCP-RUST.md` plans one; this project is the CLI only.
- No retries, pagination, domain or date filters: the API documents none.
- No structured scraping (selectors, schemas) or crawling: `ws fetch` reads one page.

## Contributing

See [AGENTS.md](AGENTS.md) for the development loop, the rules, and how to
change validated behaviour.
