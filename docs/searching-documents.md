# Searching Documents

Multi-word queries perform an `AND` search. Only documents matching all terms are returned. Results include the full stored JSON document for each match.

## Cursor-based pagination

Omit `after` for the first page, then pass the last result's `id` as `after` for subsequent pages. In `desc` mode, `after` filters IDs lower than the cursor; in `asc` mode, it filters IDs higher.

## Example

`GET /collections/{collection_name}/search?q=query_term`

Response: `200 OK`

```json
{
  "results": [
    {"id": "01HPT7B2X...", "title": "Hello", "body": "..."},
    {"id": "01HQ8C3Y...", "title": "World", "body": "..."}
  ],
  "take": 20
}
```

Search results return the full stored document for each match, including all fields that were provided at upsert time.

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

**Response:** `200 OK`
