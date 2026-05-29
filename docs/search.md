# Search

Multi-word queries perform an `AND` search. Only documents matching all terms are returned.

## Cursor-based pagination

Omit `after` for the first page, then pass the last result's `id` as `after` for subsequent pages. In `desc` mode, `after` filters IDs lower than the cursor; in `asc` mode, it filters IDs higher.

## Example

`GET /collections/{collection_name}/search?q=query_term`

Response: `200 OK`

```json
{
  "results": ["01HPT7B2X...", "01HQ8C3Y..."],
  "take": 20
}
```

Search results return object identifiers that can be resolved from your external database. This reduces storage consumption and avoids duplicate data in your main database and search engine.

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
| `results` | `array` of `string` | Matching document IDs |
| `take` | `integer` | Number of results returned |

**Response:** `200 OK`
