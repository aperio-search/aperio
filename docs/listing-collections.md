# Listing Collections

> **Auth required**: `Authorization: <main_api_key>` header.

Returns a list of all collections with their names and ID types.

## Example

::: code-group

```js [Node.js]
const { collections } = await client.listCollections();
// [{ name: "posts", id_type: "number" }, { name: "comments", id_type: "string" }]
```

```js [Fetch]
const res = await fetch("http://localhost:3000/collections", {
  headers: { Authorization: "SecretApiKey" },
});
const { collections } = await res.json();
```

```sh [cURL]
# GET /collections
#
# Response: 200 OK
# {
#   "collections": [
#     {
#       "name": "posts",
#       "id_type": "number"
#     },
#     {
#       "name": "comments",
#       "id_type": "string"
#     }
#   ]
# }

curl http://localhost:3000/collections \
  -H "Authorization: SecretApiKey"
```

:::

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `GET` |
| **Path** | `/collections` |

### Response Body

| Field | Type | Description |
|---|---|---|
| `collections` | `array` | List of collections |
| `collections[].name` | `string` | Collection name |
| `collections[].id_type` | `string` | `"number"` or `"string"` |

**Response:** `200 OK`
