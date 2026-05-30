# Deleting Collections

> **Auth required**: `Authorization: <main_api_key>` header.

Drops an entire collection index and its associated internal storage completely.

## Example

::: code-group

```js [Node.js]
await client.deleteCollection("posts");
```

```js [Fetch]
await fetch("http://localhost:3000/collections/posts", {
  method: "DELETE",
  headers: { Authorization: "SecretApiKey" },
});
```

```sh [cURL]
# DELETE /collections/{collection_name}
#
# Response: 200 OK

curl -X DELETE http://localhost:3000/collections/posts \
  -H "Authorization: SecretApiKey"
```

:::

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `DELETE` |
| **Path** | `/collections/{collection}` |

### Path Parameters

| Param | Type | Description |
|---|---|---|
| `collection` | `string` | Collection name |

**Response:** `200 OK` (no response body)
