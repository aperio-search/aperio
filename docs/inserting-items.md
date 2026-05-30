# Inserting Items

> **Auth required**: `Authorization: <main_api_key>` header.

Inserts or updates an item in the specified collection. The request body must be a JSON object with an `id` field matching the collection's `id_type`. Only fields listed in the collection's `searchable_fields` are indexed; all other fields are stored but ignored by the search index.

## Example

::: code-group

```js [Node.js]
await client.upsertItem("posts", {
  id: "01HPT7B2X",
  title: "Hello World",
  body: "Lorem ipsum dolor sit amet...",
});
```

```js [Fetch]
await fetch("http://localhost:3000/collections/posts/items", {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    Authorization: "SecretApiKey",
  },
  body: JSON.stringify({
    id: "01HPT7B2X",
    title: "Hello World",
    body: "Lorem ipsum dolor sit amet...",
  }),
});
```

```sh [cURL]
# POST /collections/{collection_name}/items
#
# Request Body:
# {
#   "id": "01HPT7B2X...",
#   "title": "Hello World",
#   "body": "Lorem ipsum dolor sit amet..."
# }
#
# Response: 200 OK

curl -X POST http://localhost:3000/collections/posts/items \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"id": "01HPT7B2X", "title": "Hello World", "body": "Lorem ipsum dolor sit amet..."}'
```

:::

> [!WARNING]
> For better performance, use sequential IDs (auto-incrementing integers, UUIDv7, ULID...). Using completely random IDs will significantly slow down the writing speed.

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `POST` |
| **Path** | `/collections/{collection}/items` |

### Request Body

The request body is an arbitrary JSON object. It **must** contain an `id` field matching the collection's `id_type`. All other fields are stored as-is. Only fields listed in the collection's `searchable_fields` param are tokenized and indexed for search.

| Field | Type | Description |
|---|---|---|
| `id` | `string` or `number` | Item ID (must match the collection's `id_type`) |
| `…` | any | Any other JSON fields; only searchable fields are indexed |

**Response:** `200 OK` (no response body)
