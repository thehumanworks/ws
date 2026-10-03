# Working on `ws`

`ws` is a Rust CLI for Cloudflare's AI Gateway Web Search API, plus `ws fetch`
for reading the pages a search finds. This file tells
coding agents (and people) how to change it safely. `CLAUDE.md` points here.

## Setup

```sh
mise install     # Rust, hk, pkl, ast-grep, Lean 4, zig, cargo-zigbuild (pinned in mise.toml)
hk install       # git hooks: pre-commit and pre-push run the checks below
```

If your shell aliases `cargo` or `sg`, bypass the alias: `command cargo ...`, and
always call `ast-grep` by its full name. `mise run <task>` is unaffected.

## The loop: change, then verify

Run this before saying a change is done. It is what the pre-commit hook and CI run.

```sh
mise run verify
```

| Step | Command | What it proves |
|---|---|---|
| Format | `mise run fmt` (fix) / `mise run fmt:check` | rustfmt-clean |
| Lint | `mise run lint` | strict clippy, warnings are errors |
| Rules | `mise run rules` | ast-grep rule tests pass, and the code obeys the rules |
| Test | `mise run test` | unit + integration tests, and agreement with the Lean model |
| Proof | `mise run proof` | Lean theorems check; `lean/vectors.txt` is current |
| Cross | `mise run cross` | Linux x86_64/arm64 and Windows binaries build (not in `verify`; run when touching dependencies or platform code) |
| Hooks | `hk check --all` / `hk fix --all` | the same checks, as git will run them |

Run one test: `cargo test --test it precedence` or `cargo test --lib validate`.

## Layout

| Path | Role |
|---|---|
| `src/main.rs` | The only place that touches the real environment, stdout, stderr |
| `src/cli.rs` | clap definitions and `run(args, env, stdout, stderr) -> exit status` |
| `src/config.rs` | Settings and precedence (`pick`): flag > environment > file > default |
| `src/validate.rs` | Bounds checked before any request is sent |
| `src/client.rs` | Wire types, the one HTTP call, response and error decoding |
| `src/fetch.rs` | `ws fetch`: one credential-free `GET` of any public page, returning a `Page` |
| `src/render.rs` | `ws fetch --render`: one Cloudflare Browser Run call (with credentials), returning a `Page` |
| `src/extract.rs` | Pure DOM rules that select a page's main content; title; HTML serialisation |
| `src/markdown.rs` | Pure HTML/DOM-to-Markdown conversion |
| `src/body.rs` | Reads a response body with a size cap that holds after decompression |
| `src/output.rs` | Text and JSON rendering, terminal sanitising |
| `src/provider.rs` | The three providers |
| `tests/it/` | Integration tests; `common.rs` is a mock HTTP server |
| `lean/` | Lean 4 model and proofs; `Vectors.lean` generates `vectors.txt` |
| `rules/`, `rule-tests/` | ast-grep rules and their test cases |
| `docs/` | Research snapshot of Cloudflare's documentation (read-only reference) |

## Rules you must keep

These are enforced by clippy (`Cargo.toml` `[lints]`) and ast-grep (`rules/`).
When a check fails, fix the code; do not weaken the check.

1. **No panics in `src/`.** No `unwrap`, `expect`, `panic!`, indexing. Return `Error`.
2. **Inject I/O.** Only `main.rs` reads `std::env` or touches stdout/stderr. Everything
   else takes `config::Env` and `&mut dyn Write`. This is why every test runs without
   real credentials.
3. **Never retry a search.** Each request is billed and has no idempotency key.
4. **Never print the token.** Not in errors, `Debug`, or logs.
5. **Count characters, not bytes** for the query limit.
6. **No `#[allow]`.** Use `#[expect(lint, reason = "...")]` if a suppression is truly needed.
7. **Document public items** (`missing_docs` is denied).
8. **Validate before sending.** Invalid input exits 2 and makes no request.
9. **Fetching is credential-free.** `src/fetch.rs`, `src/body.rs` and `src/markdown.rs`
   must never see `Credentials`, the token, or set `Authorization`/`Cookie`: `ws fetch`
   talks to arbitrary hosts and follows redirects. The one exception is
   `src/render.rs`, a Cloudflare client like `client.rs`: it sends the token to
   Cloudflare only, never follows redirects, and is opt-in (`--render`).
10. **Never render automatically or in a loop.** Browser Run time is metered; `--render`
    is the only trigger and it makes exactly one request.
11. **Fetch only public addresses by default.** The resolver in `fetch.rs` refuses
    non-public addresses on every hop; `--allow-private` is the only way around it.
12. **Keep the page pipeline separate.** Retrieval (`fetch.rs`, `render.rs`) produces a
    `fetch::Page`; selection (`extract.rs`) and conversion (`markdown.rs`) are pure. New
    consumers such as structured scraping take a `Page` or `extract::Document` and live
    in their own module. Extraction rules are fixed lists, not scores: change the lists
    and their tests together, and keep `--raw` meaning "nothing removed".

## Tests

- Add a unit test next to the code and, for behaviour visible to users, an
  integration test in `tests/it/cli.rs` (drive `ws::cli::run` with a fake
  environment and the mock server).
- Tests must never contact Cloudflare. Use `MockServer` and `WS_API_BASE_URL`
  (accepted only for loopback addresses).
- Tests of `ws fetch` (`tests/it/fetch.rs`) must never contact the internet either:
  point them at `MockServer` with `Canned::page` and pass `--allow-private` (the
  `run` helper does). `--render` tests use `WS_API_BASE_URL` like search.
- When you observe a new response or error shape from the live API, add the body
  as a test fixture in `src/client.rs` with the date.

## Changing validation or precedence

These are specified in Lean and the Rust code is tested against the model. That
includes the `ws fetch` rules in `lean/WsSpec/Fetch.lean`: URL validation, which
IPv4 addresses are public, and how a media type is classified.

1. Change the definitions and theorems in `lean/WsSpec/`. No `sorry`.
2. `mise run proof:vectors` to regenerate `lean/vectors.txt`.
3. Change the Rust code until `cargo test --test it lean_vectors` passes.
4. `mise run proof` confirms the proofs check and the vectors file is current.

## Adding an ast-grep rule

Add `rules/<id>.yml` and `rule-tests/<id>-test.yml` with `valid` and `invalid`
cases, then `mise run rules`. Explain the reason in the rule's `note`; that text
is what the next agent sees when the rule fires.

## Live checks

Credentials come from the environment (`CLOUDFLARE_API_TOKEN`,
`CLOUDFLARE_ACCOUNT_ID`; on the maintainer's machine `fnox` exports them).
Never write them to a file, a test, or the transcript. Live searches cost money
and `--render` uses metered browser time: run the fewest needed (searches with
`--limit 1`), and say what you ran.

```sh
cargo run --release -- search "cloudflare workers" --limit 1
cargo run --release -- fetch https://example.com   # free: no gateway, no credentials
cargo run --release -- fetch https://example.com --render   # metered Browser Run time
demo/demo.sh --mock     # full tour with no network or billing
```

## Commits

Hooks run on commit and push. Do not bypass them with `--no-verify`. Commit
`Cargo.lock`, `lean/lake-manifest.json` and `lean/vectors.txt`.
