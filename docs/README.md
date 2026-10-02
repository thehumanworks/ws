# `ws` implementation documentation

Primary-source research snapshot collected **2026-10-02**, starting from [Cloudflare Web Search `llms.txt`](https://developers.cloudflare.com/web-search/llms.txt). All four pages indexed there are saved locally, along with relevant Gateway setup, billing, authentication and logging references. This folder is an implementation handoff for an AI agent building a Rust CLI and MCP server named `ws`; it contains documentation, not a completed binary.

Read in order:

1. [Cloudflare API contract](CLOUDFLARE-WEB-SEARCH.md): exact endpoint, token scopes, nested request fields, normalized response, provider pricing/modes, BYOK precedence, limitations and unresolved live checks.
2. [Rust CLI delivery plan](RUST-CLI.md): proposed commands/configuration, shared client, Rust serialization, errors, paid retries and acceptance tests.
3. [Rust MCP guidance](MCP-RUST.md): current protocol/SDK, stdio transport, search tool schema, client configuration and end-to-end checks.
4. [Source manifest](sources/manifest.json): retrieval URLs, timestamps and SHA-256 hashes for the saved primary sources.

Machine-readable API schema extractions:

- [REST request](schemas/search-request.schema.json)
- [Response](schemas/search-response.schema.json)

Schemas reproduce the published MDX constraints; titles and `$schema` are local additions. They intentionally do not add `additionalProperties: false` or top-level response requirements absent from upstream. An implementation should still reject an unexpected success response missing `items` rather than report fabricated empty results. JSON Schema defaults are annotations, not automatic insertion.

Local snapshots retain Cloudflare page boilerplate for provenance. `sources/cloudflare/how-to-use.mdx` contains the expanded schema that the rendered Markdown collapses. Supplementary generic Gateway pages provide context; their headers/features are not automatically guaranteed for the new search endpoint.

No authenticated search calls were made, so endpoint accessibility, credentials, billing and live provider behavior remain unverified. Build and mock tests alone cannot establish those facts. Future implementers should resolve the explicit live-check list and re-check beta API documentation before release.
