# Searching Documents

> **Auth required**: `Authorization: <main_api_key>` or `Authorization: <search_api_key>` header.

Multi-word queries perform an `AND` search. Only documents matching all terms are returned. Results include the full stored JSON document for each match.

## Cursor-based pagination

Omit `after` for the first page, then pass the last result's `id` as `after` for subsequent pages. In `desc` mode, `after` filters IDs lower than the cursor; in `asc` mode, it filters IDs higher.

## Example

Search results return the full stored document for each match, including all fields that were provided at upsert time.

::: code-group

```js [Node.js]
const { results, take, elapsed_ms } = await client.search("posts", {
  q: "hello world",
  sort: "asc",
  take: 50,
  after: "01HQ8C3Y",
});
```

```js [Fetch]
const params = new URLSearchParams({
  q: "hello world",
  sort: "asc",
  take: "50",
  after: "01HQ8C3Y",
});
const res = await fetch(`http://localhost:3000/collections/posts/search?${params}`, {
  headers: { Authorization: "SecretApiKey" },
});
const { results, take, elapsed_ms } = await res.json();
```

```sh [cURL]
# GET /collections/{collection_name}/search?q=query_term
#
# Response: 200 OK
# {
#   "results": [
#     {"id": "01HPT7B2X...", "title": "Hello", "body": "..."},
#     {"id": "01HQ8C3Y...", "title": "World", "body": "..."}
#   ],
#   "take": 20,
#   "elapsed_ms": 1.234
# }

curl "http://localhost:3000/collections/posts/search?q=hello+world&sort=asc&take=50&after=01HQ8C3Y" \
  -H "Authorization: SecretApiKey"
```

:::

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `GET` |
| **Path** | `/collections/{collection}/search` |

### Query Parameters

| Param | Type | Default | Description |
|---|---|---|---|
| `q` | `string` | — | Search query (one or more terms, space-separated) |
| `sort` | `string` | `desc` | Sort by ID: `asc` or `desc` |
| `take` | `integer` | `20` | Max results (clamped 1 – 100) |
| `after` | `string` | — | Exclusive cursor ID for cursor-based pagination |

### Response Body

| Field | Type | Description |
|---|---|---|
| `results` | `array` of `object` | Matching documents (full stored JSON objects) |
| `take` | `integer` | Number of results returned |
| `elapsed_ms` | `number` | Time spent executing the search, in milliseconds |

**Response:** `200 OK`
