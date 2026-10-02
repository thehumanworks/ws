# Cloudflare Web Search API: implementation contract for `ws`

Retrieved 2026-10-02. Product status: open beta. This is an extraction of primary Cloudflare documentation, with gaps explicitly identified. Original pages are saved in `sources/cloudflare/`. Begin with this file, then `RUST-CLI.md` and `MCP-RUST.md`. Proposed application behavior in those files is a recommendation, not an upstream API guarantee.

## Endpoint and authentication

Use REST from native Rust; no Worker deployment, Workers AI model inference, provider SDK, or LLM API key is required for search.

```text
POST https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/websearch/
Authorization: Bearer <CLOUDFLARE_API_TOKEN>
Content-Type: application/json
```

Use a Cloudflare account ID, not a zone ID. Create a custom API token scoped to the target account with **Account → Workers AI → Read** and **Account → AI Gateway → Read**. The search-specific guide requires these two Read permissions; broader gateway administration guides also request Edit for setup. The runtime should not need Edit merely to search. A successful token verification does not prove account scope, both permissions, billing, or search access.

Every request specifies an AI Gateway. The search guide says accounts have `default`; gateway setup explains default gateways may be created on the first authenticated request. Verify the intended gateway in the dashboard rather than relying on undocumented search-specific creation behavior. Credits must be loaded or a provider key must be stored on that gateway.

Sources: [search usage](https://developers.cloudflare.com/web-search/how-to-use/), [gateway setup](https://developers.cloudflare.com/ai-gateway/get-started/), [token creation](https://developers.cloudflare.com/fundamentals/api/get-started/create-token/), [account ID](https://developers.cloudflare.com/fundamentals/account/find-account-and-zone-ids/).

## Complete REST request

```json
{
  "query": "What is Cloudflare Workers?",
  "provider": "ceramic",
  "limit": 5,
  "byokAlias": "default",
  "options": { "gateway": { "id": "default" } }
}
```

`byokAlias` above is optional; omit it when default-key-or-credits behavior is intended.

| JSON path | Type | Required | Constraint/default |
|---|---|---|---|
| `query` | string | yes | 1–1,024 characters |
| `provider` | string | no | `ceramic`, `exa`, `linkup`; default `ceramic` |
| `limit` | integer | no | 1–10 inclusive; default 10 |
| `byokAlias` | string | no | `^[A-Za-z0-9_-]{1,64}$` |
| `options` | object | yes | contains `gateway` |
| `options.gateway` | object | yes | contains `id` |
| `options.gateway.id` | string | yes | gateway identifier |

The rendered Markdown corrupts the alias pattern with escaping. The original MDX schema confirms the pattern above and the required nested gateway fields. See [upstream MDX](https://github.com/cloudflare/cloudflare-docs/blob/production/src/content/docs/web-search/how-to-use.mdx) and local `sources/cloudflare/how-to-use.mdx`.

No search pagination, cursor, offset, streaming, domain filters, language/region selector, date filters, full-page extraction, answer generation, search-depth setting, or custom provider options are documented. Do not advertise direct Exa or Linkup API features as supported by this wrapper. Search snippets and page metadata are the available grounding context; fetching pages is a separate feature.

## Normalized response

The documented response is a direct object, not a demonstrated Cloudflare `result/success/errors` envelope:

```json
{
  "items": [
    {
      "url": "https://example.com/page",
      "title": "Example page",
      "description": "Relevant excerpt",
      "imageUrl": "https://example.com/image.png",
      "faviconUrl": "https://example.com/favicon.ico",
      "lastModifiedDate": "2026-10-02T12:00:00Z"
    }
  ],
  "metadata": {
    "query": "What is Cloudflare Workers?",
    "requestId": "<REQUEST_ID>",
    "latencyMs": 612
  }
}
```

This is a synthesized shape illustration, not an observed authenticated response. Official usage includes an example with `items` and `metadata`; additional optional fields come from the original MDX schema.

| Path | Type | Required by published schema |
|---|---|---|
| `items` | array of objects | top-level schema does not declare required fields |
| `items[].url` | string | yes, per item |
| `items[].title` | string | yes, per item |
| `items[].description` | string | no |
| `items[].imageUrl` | string | no |
| `items[].faviconUrl` | string | no |
| `items[].lastModifiedDate` | string, date-time | no |
| `metadata` | object | no |
| `metadata.query` | string | no |
| `metadata.requestId` | string | no |
| `metadata.latencyMs` | number | no; do not assume integer |

Only optional fields returned by a provider are included. Keep missing descriptions distinct from empty strings. Preserve source URLs for citation. A last-modified date is not necessarily a publication date. Do not impose URL or date parsing failures on otherwise usable results unless required by a downstream feature. Ignore or retain unknown fields for forward compatibility; beta APIs may evolve. Missing `items` should be handled deliberately rather than silently treated as a successful empty search.

Source: [usage/response](https://developers.cloudflare.com/web-search/how-to-use/#response-format), original MDX above.

## Billing and BYOK selection

| Request configuration | Documented outcome |
|---|---|
| Explicit `byokAlias` and matching stored provider key | Stored key used; provider bills directly |
| Explicit alias but provider/alias missing | HTTP 400; no fallback to Gateway credits |
| Alias omitted, provider has stored `default` key | Stored `default` key used |
| Alias omitted, no stored `default` key | AI Gateway credits used |

Store provider keys through AI Gateway → select gateway → Provider Keys. Add Ceramic.ai, Exa, or Linkup and assign an alias. If necessary use Configure custom providers. The search request carries the alias, not the provider secret. Cloudflare still requires its own API token. Stored keys are encrypted with Secrets Store.

The general Gateway credential/header rules concern other inference and provider-passthrough endpoints. Use search's documented JSON `byokAlias`, not an invented direct provider `Authorization` header or `cf-aig-byok-alias` substitution. There is no documented search parameter forcing credits while ignoring a stored default key.

Search prices are provider list prices with no per-search markup. Separately, Unified Billing documentation states a **5% fee on credit purchases** ($100 credits costs $105). Do not describe funded search as entirely fee-free. Load credits in AI Gateway → Manage → Top-up credits; this is operator setup, not something `ws` should automatically purchase.

Sources: [search BYOK](https://developers.cloudflare.com/web-search/how-to-use/#bring-your-own-key-byok), [billing](https://developers.cloudflare.com/ai-gateway/features/unified-billing/).

## Providers and tradeoffs

| Provider value | Price per 1,000 requests | Upstream behavior in this API | Provider ZDR listed |
|---|---:|---|---|
| `ceramic` | $0.25 | Default; independent >40 billion-page index; descriptions up to 8,000 characters | yes |
| `exa` | $7.00 | `auto` search type; highlights normalized into description | yes |
| `linkup` | $5.00 | `fast` search depth; raw results, no generated answer | yes |

Prices are a dated snapshot and can change. No free allowance is specified in search docs. No guarantee of exact result count or fixed latency is given. Ceramic's long snippets mean MCP output can be large even with ten results. Any truncation should be an explicit client presentation policy, with URLs retained and truncation disclosed.

All providers commit to verified-bot requirements, crawler identification, respecting `robots.txt`, and source attribution. Provider Zero Data Retention does **not** imply Cloudflare does not log the request.

Sources: [providers](https://developers.cloudflare.com/web-search/providers/), [about/crawler standards](https://developers.cloudflare.com/web-search/about/).

## Gateway observability and limits

Search requests are recorded in Gateway logs/analytics alongside inference traffic. Generic Gateway logging is enabled by default. New customers whose first gateway was created on/after 2026-09-24 use Workers Logs pricing/retention; earlier customers use Legacy Logs. Queries and responses may be sensitive. Operator setup should review gateway logging policy.

Generic Gateway docs describe `cf-aig-collect-log: false` and `cf-aig-collect-log-payload: false`. Search docs do not explicitly confirm forwarding/acceptance of these headers; do not claim a working per-search privacy flag until verified. Likewise general caching, dynamic routing, spending controls and rate-limit settings do not establish an endpoint-specific wire contract.

Documented search limits: query ≤1,024 characters; limit ≤10. There is **no numeric request-per-second quota, timeout SLA, Retry-After guarantee, cache behavior, or error-code catalog** in the four search pages. Do not substitute general Workers AI model rate limits for search limits.

Sources: [about](https://developers.cloudflare.com/web-search/about/), [logging](https://developers.cloudflare.com/ai-gateway/observability/logging/), [generic rate limiting](https://developers.cloudflare.com/ai-gateway/features/rate-limiting/).

## Workers binding: comparison only

`env.AI.websearch({ gatewayId, query, provider?, limit?, byokAlias? })` returns a standard `Response`; read with `.json()`. Binding configuration is `[ai] binding = "AI"` in Wrangler and a current compatibility date. REST requires nested `options.gateway.id`; the binding instead requires top-level `gatewayId`. Do not send binding-shaped JSON to REST. Native `ws` needs no Wrangler configuration.

## Gaps to resolve during implementation acceptance

1. Confirm authenticated response shape from this REST endpoint; do not infer envelope shape from unrelated APIs.
2. Record actual sanitized error bodies/statuses for invalid input, missing alias, denied scope, and unavailable funding. Only missing-alias 400 is explicitly documented here.
3. Verify Unicode boundary behavior. Docs say characters, not bytes; count Rust Unicode scalar values initially and test non-ASCII/emoji rather than using `String::len()`.
4. Verify each configured provider and gateway with live search, preserving request IDs and sources. Successful mock tests do not prove credentials or provider availability.
5. Determine actual rate-limit behavior before promising automatic retry or gateway header support. Paid POST retries can duplicate charges; no idempotency key is documented.

No authenticated searches were run while preparing this reference. No secrets were requested or saved.
