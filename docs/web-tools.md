# Web tools: `web_fetch` and `web_search`

gi ships two model-callable tools for the web:

- **`WebFetch`** — fetch a URL, convert it to readable text, and answer a prompt
  about it. **Keyless** — works out of the box.
- **`WebSearch`** — search the web and return cited results. The search **backend
  is pluggable**; pick one via config or environment.

Both are network reads, so they require `workspace-write` permission: in
`default` mode the agent asks before using them, in `edit`/`mugen` they run
automatically, and in strict `read-only` they're declined.

## Search backends

`web_search` supports four backends, selected by `GI_SEARCH_PROVIDER`:

| Provider     | `GI_SEARCH_PROVIDER` | Key needed | Notes |
|--------------|----------------------|------------|-------|
| Brave        | `brave`              | yes        | `GI_SEARCH_API_KEY` → `X-Subscription-Token` |
| Tavily       | `tavily`             | yes        | `GI_SEARCH_API_KEY` (AI-friendly results) |
| SearXNG      | `searxng`            | no         | `GI_SEARCH_BASE_URL` = your instance (uses `?format=json`) |
| DuckDuckGo   | *(unset)* / `duckduckgo` | no     | keyless HTML scrape — the default fallback |

Environment variables (read by the `web_search` tool):

- `GI_SEARCH_PROVIDER` — `brave` \| `tavily` \| `searxng` \| `duckduckgo`.
- `GI_SEARCH_API_KEY` — API key for Brave/Tavily.
- `GI_SEARCH_BASE_URL` — override/endpoint (required for SearXNG; optional for
  Brave/Tavily to point at a proxy).
- `GI_WEB_SEARCH_BASE_URL` — legacy: a generic HTML search endpoint, scraped.

A key-required backend with no key returns a clear, actionable error instead of
failing silently. `web_fetch` never needs a key.

## Configure via settings.json

Add a `web_search` block to `~/.gi/settings.json` (user-level) or
`.gi/settings.json` (project-level; project wins). gi applies it to the
`GI_SEARCH_*` environment at startup. An env var you export yourself always wins.

```json
{
  "web_search": {
    "provider": "searxng",
    "base_url": "http://192.168.50.3:8888"
  }
}
```

Brave / Tavily example:

```json
{
  "web_search": { "provider": "brave", "api_key": "BSA..." }
}
```

## Quick check

```sh
# SearXNG
GI_SEARCH_PROVIDER=searxng GI_SEARCH_BASE_URL=http://192.168.50.3:8888 \
  gi "search the web for the latest ratatui release and cite sources"

# Fetch a page (no key)
gi "fetch https://doc.rust-lang.org/std/ and summarize what it covers"
```
