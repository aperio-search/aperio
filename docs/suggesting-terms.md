# Suggesting Terms

> **Auth required**: `Authorization: <main_api_key>` or `Authorization: <search_api_key>` header.

The suggest endpoint returns indexed terms that match a given prefix.

## Word-Level Matching

Suggest operates at the **individual word level**. When you pass a multi-word query like `"hello wo"`, only the last word (`"wo"`) is used as the prefix for matching against the FST. The FST stores single lemmatized tokens from your documents (e.g. `hello`, `world`, `application`), so suggestions are always whole words, never phrases.

Multi-word query examples:

| Input | Prefix used | Matches |
|---|---|---|
| `appl` | `appl` | `apple`, `application`, … |
| `hello app` | `app` | `apple`, `application`, … |
| `search wo` | `wo` | `word`, `world`, `wonder`, … |

## Example

::: code-group

```js [Node.js]
const { results } = await client.suggest("posts", {
  q: "hello app",
  take: 5,
});
// results → ["apple", "application", "appliance"]
```

```js [Fetch]
const params = new URLSearchParams({
  q: "hello app",
  take: "5",
});
const res = await fetch(
  `http://localhost:3000/collections/posts/suggest?${params}`,
  { headers: { Authorization: "SecretApiKey" } },
);
const { results, take } = await res.json();
```

```sh [cURL]
curl "http://localhost:3000/collections/posts/suggest?q=hello+app&take=5" \
  -H "Authorization: SecretApiKey"

# Response: 200 OK
# {
#   "results": ["apple", "application", "appliance"],
#   "take": 5
# }
```

:::

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `GET` |
| **Path** | `/collections/{collection}/suggest` |

### Query Parameters

| Param | Type | Default | Description |
|---|---|---|---|
| `q` | `string` | — | Prefix to match against indexed terms (only the last word is used) |
| `take` | `integer` | `10` | Max suggestions (clamped 1 – 100) |

### Response Body

| Field | Type | Description |
|---|---|---|
| `results` | `array` of `string` | Matching indexed terms (single words, not phrases) |
| `take` | `integer` | Number of suggestions returned |

**Response:** `200 OK`

## Notes

- The FST is a **best-effort** vocabulary index. Terms are batched during index queue processing and consolidated to disk periodically (configurable via `fst_consolidate_interval_secs`).
- Terms from deleted documents may remain in the FST until the next full consolidation; this is an eventual-consistency trade-off.
- Matching is **prefix-based only** — partial infix or substring matching is not supported.
