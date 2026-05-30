# Viewing Collection Metadata

> **Auth required**: `Authorization: <main_api_key>` header.

Returns metadata about the collection, including the number of indexed documents and unique terms in the inverted index.

## Example

::: code-group

```js [Node.js]
const meta = await client.getCollection("posts");
// { name: "posts", id_type: "number", document_count: 42, unique_terms: 318 }
```

```js [Fetch]
const res = await fetch("http://localhost:3000/collections/posts", {
  headers: { Authorization: "SecretApiKey" },
});
const meta = await res.json();
```

```sh [cURL]
# GET /collections/{collection_name}
#
# Response: 200 OK
# {
#   "name": "posts",
#   "id_type": "number",
#   "document_count": 42,
#   "unique_terms": 318
# }

curl http://localhost:3000/collections/posts \
  -H "Authorization: SecretApiKey"
```

:::

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `GET` |
| **Path** | `/collections/{collection}` |

### Path Parameters

| Param | Type | Description |
|---|---|---|
| `collection` | `string` | Collection name |

### Response Body

| Field | Type | Description |
|---|---|---|
| `name` | `string` | Collection name |
| `id_type` | `string` | `"number"` or `"string"` |
| `document_count` | `integer` | Number of indexed documents |
| `unique_terms` | `integer` | Number of unique terms |

**Response:** `200 OK`
