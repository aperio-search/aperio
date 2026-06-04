# Bulk Ingestion

> **Auth required**: `Authorization: <main_api_key>` header.

Inserts or updates multiple items in a single request. This is significantly faster than sending individual requests, especially for large datasets.

## Example

::: code-group

```js [Fetch]
await fetch("http://localhost:3000/collections/posts/items/bulk", {
  method: "POST",
  headers: {
    "Content-Type": "application/json",
    Authorization: "SecretApiKey",
  },
  body: JSON.stringify([
    { id: "01HPT7B2X", title: "Hello World", body: "Lorem ipsum..." },
    { id: "01HPT7B2Y", title: "Second Post", body: "Dolor sit amet..." },
  ]),
});
```

```sh [cURL]
# POST /collections/{collection_name}/items/bulk
#
# Request Body:
# [
#   { "id": "01HPT7B2X", "title": "Hello World", "body": "Lorem ipsum..." },
#   { "id": "01HPT7B2Y", "title": "Second Post", "body": "Dolor sit amet..." }
# ]
#
# Response: 200 OK
# {
#   "ok": true,
#   "count": 2
# }

curl -X POST http://localhost:3000/collections/posts/items/bulk \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '[
    {"id": "01HPT7B2X", "title": "Hello World", "body": "Lorem ipsum..."},
    {"id": "01HPT7B2Y", "title": "Second Post", "body": "Dolor sit amet..."}
  ]'
```

:::

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `POST` |
| **Path** | `/collections/{collection}/items/bulk` |

### Request Body

| Field | Type | Description |
|---|---|---|
| (body) | `array` | Array of JSON objects, each with an `id` field matching the collection's `id_type` |

### Response Body

| Field | Type | Description |
|---|---|---|
| `ok` | `boolean` | Always `true` on success |
| `count` | `number` | Number of items queued for indexing |
