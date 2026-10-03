#!/usr/bin/env bash
# A guided tour of ws.
#
#   demo/demo.sh --mock   run against a local mock server: no credentials, no billing
#   demo/demo.sh          run against Cloudflare: needs CLOUDFLARE_API_TOKEN and
#                         CLOUDFLARE_ACCOUNT_ID, and a funded AI Gateway (searches are billed);
#                         its one `fetch --render` uses metered Browser Run time and needs
#                         Browser Rendering: Edit on the token
set -euo pipefail
cd "$(dirname "$0")/.."

mode="${1:-live}"
cargo build --release --quiet
ws="$PWD/target/release/ws"

# Keep the demo away from your real saved defaults.
tmp="$(mktemp -d)"
export WS_CONFIG="$tmp/config.toml"
unset WS_PROVIDER WS_GATEWAY_ID WS_LIMIT WS_BYOK_ALIAS WS_TIMEOUT_SECS WS_API_BASE_URL
cleanup() {
  if [ -n "${server_pid:-}" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

if [ "$mode" = "--mock" ]; then
  port=18787
  python3 demo/mock_server.py "$port" &
  server_pid=$!
  sleep 0.5
  export WS_API_BASE_URL="http://127.0.0.1:$port"
  export CLOUDFLARE_ACCOUNT_ID="0123456789abcdef0123456789abcdef"
  export CLOUDFLARE_API_TOKEN="demo-token"
fi

# The page `ws fetch` reads: the mock serves one locally, live mode uses example.com.
# The mock is on loopback, which fetch refuses unless told otherwise.
page="https://example.com/"
private=""
if [ "$mode" = "--mock" ]; then
  page="http://127.0.0.1:$port/example/1"
  private=" --allow-private"
fi

say() { printf '\n\033[1m# %s\033[0m\n' "$*"; }
# Print a command, run it, and show its exit status without stopping the tour.
run() {
  printf '$ %s\n' "$*"
  set +e
  eval "${*//ws /$ws }"
  status=$?
  set -e
  [ "$status" -eq 0 ] || printf '[exit status %s]\n' "$status"
}

say "The three providers Cloudflare offers; * marks the one ws will use"
run "ws providers"

say "Search with the default provider (Ceramic) and gateway ('default')"
run "ws search what is cloudflare workers --limit 3"

say "Pick a provider for one search with a flag"
run "ws search 'rust async runtimes' --provider exa --limit 2"

say "JSON for scripts and agents"
run "ws search 'rust async runtimes' --provider linkup --limit 1 --json"

say "Read a page a search found: its main content as Markdown, no credentials, nothing billed"
run "ws fetch $page$private"

say "--raw keeps the whole page: header, navigation, sidebar and footer included"
run "ws fetch $page$private --raw"

say "The main content as JSON (URLs, status, title, Markdown) or as HTML"
run "ws fetch $page$private --format json"
run "ws fetch $page$private --format html"

say "Hosts that are not on the public internet are refused unless --allow-private is given"
run "ws fetch http://127.0.0.1:9/"

say "--render loads the page in Cloudflare Browser Run first, so JavaScript content is included (metered)"
run "ws fetch $page --render"

say "Save a different default provider"
run "ws config set provider exa"
run "ws config show"

say "The environment overrides the saved default..."
run "WS_PROVIDER=linkup ws config show"
run "WS_PROVIDER=linkup ws search precedence -n 1"

say "...and a flag overrides the environment"
run "WS_PROVIDER=linkup ws search precedence -n 1 --provider ceramic"

say "Choose the AI Gateway with an environment variable or a flag"
run "WS_GATEWAY_ID=team-gateway ws search gateways -n 1"
run "WS_GATEWAY_ID=team-gateway ws search gateways -n 1 --gateway other-gateway"

say "Mistakes are caught locally, before anything is billed (exit status 2)"
run "ws search too many results --limit 11"
run "ws search oops --provider google"

if [ "$mode" = "--mock" ]; then
  say "API failures explain themselves (exit status 1)"
  run "ws search anything --gateway unfunded"
fi
