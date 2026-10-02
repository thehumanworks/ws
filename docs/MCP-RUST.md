# Rust MCP implementation guidance

`ws mcp` should expose the same Cloudflare search operation as the CLI through a local MCP stdio server. This is an implementation recommendation, not an implemented or tested server. Source snapshots were retrieved on 2026-10-02; [sources/mcp/manifest.json](sources/mcp/manifest.json) records URLs, SHA-256 hashes, and pinned SDK commit IDs.

## Protocol and SDK baseline

The current specification is [MCP 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28), which uses per-request metadata instead of a connection initialization handshake. Every modern request needs `_meta["io.modelcontextprotocol/protocolVersion"]` and `_meta["io.modelcontextprotocol/clientCapabilities"]` inside `params`; client identity is optional. Servers implement `server/discover`, and complete results include `resultType: "complete"`. [Base protocol](https://modelcontextprotocol.io/specification/2026-07-28/basic), [discovery](https://modelcontextprotocol.io/specification/2026-07-28/server/discover).

Keep compatibility with 2025-11-25 hosts: client `initialize` → negotiated server version/capabilities/identity → client `notifications/initialized` → `tools/list` and `tools/call`. Modern clients can probe `server/discover`; no `notifications/initialized` follows modern discovery. Advertise only versions actually supported by the chosen SDK and verified in tests. [Legacy lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle), [current compatibility rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning).

Use the official [Rust SDK, rmcp](https://github.com/modelcontextprotocol/rust-sdk). Its captured stdio server implementation recognizes both legacy initialization and requests with modern metadata; retain that behavior. Select and lock a published release whose API and protocol support match the implementation. The pinned repository workspace declares `3.5.0` and Rust `1.88`; this establishes source requirements, not that a particular registry version was resolved or built here. [Captured workspace](sources/mcp/rust-sdk-cargo.toml), [server implementation](sources/mcp/rmcp-server.rs).

Enable rmcp `server`, `macros`, and `transport-io`; use Tokio, Serde, and Schemars for async execution, parameter decoding, and schemas. A handler using `#[tool_router]`, `#[tool]`, `Parameters<T>`, and an explicit `#[tool_handler]`/`ServerHandler` can declare `ws` identity and the tools capability. Serve with `transport::stdio()` and await the service's completion. These APIs are illustrated in the [official SDK README](https://github.com/modelcontextprotocol/rust-sdk#tools); check the resolved release rather than copying an example from a different SDK generation.

## One tool and one shared operation

Recommended tool name: `web_search`. Description: “Search the public web through the Cloudflare Web Search API. Returns source URLs and titles, with descriptions and metadata when supplied. Sends the query to the configured provider; does not fetch result pages.”

Recommended advertised input schema:

```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "minLength": 1,
      "maxLength": 1024,
      "description": "Public-web search query."
    },
    "provider": {
      "type": "string",
      "enum": ["ceramic", "exa", "linkup"],
      "description": "Optional override of the operator-configured provider."
    },
    "limit": {
      "type": "integer",
      "minimum": 1,
      "maximum": 10,
      "description": "Maximum number of returned items."
    }
  },
  "required": ["query"],
  "additionalProperties": false
}
```

Implement these constraints in runtime validation as well as schema generation: an absent limit uses the resolved operator `WS_LIMIT` (default 10), unknown fields must be rejected, and blank queries, Unicode character counting, provider selection, and numeric limits need explicit checks. Omit a static schema default when operator configuration can change it, or advertise the actual resolved value. Gateway ID, account ID, Cloudflare authentication, provider credentials/BYOK alias, endpoint, and timeout are operator configuration. Keep them outside model-supplied arguments. Choose the same default provider as the CLI; never silently switch providers after an error. [Shared configuration](RUST-CLI.md).

The MCP handler should call a shared `search` service directly, using the same configuration, request construction, response decoding, and error mapping as `ws search`. Reuse one HTTP client and bounded request concurrency. Apply a deadline to the complete operation; cancellation should stop the HTTP future, including pending retry waits. Search may incur charges, so retries must follow an explicit bounded policy.

Set `readOnlyHint: true`, `destructiveHint: false`, and `openWorldHint: true`. Read-only means the operation does not modify user data; it still sends a query externally and may consume quota. For a fixed single-tool catalog, omit tool-list-change support. Do not advertise resources, prompts, tasks, subscriptions, or elicitation unless implemented. Tool annotations are hints, not authorization controls. [Tools specification](https://modelcontextprotocol.io/specification/2026-07-28/server/tools).

## Results and errors

Return success as an object in `structuredContent`, preserving the Cloudflare `items` and `metadata` shape used by CLI JSON. Each item has `url` and `title`; retain supplied optional `description`, `imageUrl`, `faviconUrl`, and `lastModifiedDate`. Preserve upstream optional metadata fields without inventing a request ID or latency measurement. Forward `limit` upstream and preserve the returned object; if upstream returns more items than requested, choose and document an explicit policy rather than silently truncating. An empty item list is a successful search. [Upstream contract and credentials](CLOUDFLARE-WEB-SEARCH.md).

Also return a `content` text block containing the serialized result object so clients without structured-result support receive the same data. If declaring `outputSchema`, every structured result must conform. Use an object schema for interoperability with 2025-11-25 clients, even though current MCP permits other JSON result types. [Current structured results](https://modelcontextprotocol.io/specification/2026-07-28/server/tools#structured-content), [legacy tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).

Recommended error distinction:

| Failure | Response |
| --- | --- |
| Unknown tool, malformed JSON-RPC/request envelope, missing modern protocol metadata | SDK JSON-RPC protocol error |
| Well-formed call with invalid search values, provider authentication failure, rate limit, upstream failure, timeout, bad upstream response | Tool result with `isError: true` and actionable text |
| Missing operator configuration at launch | Nonzero process exit; brief redacted diagnostic on stderr |

The tool error text can serialize `{ "code": "rate_limited", "message": "Search provider rate limit reached.", "retryable": true }`, with optional HTTP status and parsed retry delay when known. This is a proposed local error contract. To return it in `structuredContent`, declare an output-schema union covering both success and error objects; otherwise keep errors in text content. Do not return raw response bodies, authorization headers, tokens, or queries in diagnostic logs. SDK tool failures use `Ok(CallToolResult::error(...))`; `Err(ErrorData)` is the protocol-error path. [SDK errors](https://github.com/modelcontextprotocol/rust-sdk#error-handling), [protocol/tool error rules](https://modelcontextprotocol.io/specification/2026-07-28/server/tools#error-handling).

## Stdio and client integration

MCP stdio carries one UTF-8 JSON-RPC message per line. Stdout must contain only MCP messages; logging belongs on stderr. Therefore `ws mcp` must bypass the CLI's normal result printing, progress bars, banners, and pretty JSON. Initialize tracing with a stderr writer. Newlines inside string values must be JSON-escaped. EOF on stdin should terminate promptly. [Stdio requirements](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio).

After implementation and installation, a Claude Desktop-style launch entry is:

```json
{
  "mcpServers": {
    "ws": {
      "command": "/absolute/path/to/ws",
      "args": ["mcp"]
    }
  }
}
```

Use an absolute binary path and ensure the launched process receives the required operator configuration. A GUI host may not inherit the terminal environment. Supply credentials through the approved runtime secret mechanism rather than committing literal values to client configuration. The `mcpServers` format is host configuration, not part of the MCP wire protocol. [Official host setup example](https://modelcontextprotocol.io/docs/2026-07-28/develop/build-server).

For Codex, the documented stdio configuration is:

```toml
[mcp_servers.ws]
command = "/absolute/path/to/ws"
args = ["mcp"]
env_vars = ["CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID", "WS_GATEWAY_ID", "WS_PROVIDER", "WS_LIMIT", "WS_BYOK_ALIAS", "WS_TIMEOUT_SECS"]
```

The proposed `WS_*` names are defined in [shared configuration](RUST-CLI.md). `env_vars` forwards runtime configuration without storing secret values in the file. Set `tool_timeout_sec` above the server's complete-operation deadline. [Official Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

## Acceptance checks for the implementation

1. Start the actual release binary as a child process with a mock search backend. Exercise both modern discovery/per-request metadata and the 2025-11-25 initialization sequence; verify negotiated versions, identity, and only the tools capability.
2. Assert `tools/list` returns exactly `web_search` with the declared schema and annotations. Test missing/blank/overlong Unicode query, unknown fields/provider, absent limit, and limits 0/1/10/11 without an upstream call for invalid inputs.
3. Compare CLI JSON and MCP structured success for identical backend fixtures, including zero items, absent optional metadata, operator-default limits, unexpected excess items, Unicode, and embedded newlines. Validate every returned object against the advertised output schema.
4. Exercise upstream 401/403/429/5xx, timeout, malformed JSON, and Cloudflare envelope failure. Assert useful `isError` results, redaction, bounded retries, and no provider substitution.
5. Capture stdout and stderr separately across startup, successful calls, errors, concurrent calls, cancellation, and EOF. Every stdout line must parse as a valid MCP message with correctly correlated IDs; diagnostics must not contain secrets. Verify cancelled work stops and EOF exits promptly.
6. Connect one real intended MCP host using the installed binary and operator configuration; discover and call `web_search`. Record this separately from mock-backed checks and from any live Cloudflare/provider authorization test.

No Rust binary, dependency resolution, protocol test, client integration, or live provider call was performed for this documentation task.
