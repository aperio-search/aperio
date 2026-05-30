# Deleting Items

> **Auth required**: `Authorization: <main_api_key>` header.

Deletes a single item from a collection by its `ID`.

## Example

::: code-group

```js [Node.js]
await client.deleteItem("posts", "01HPT7B2X");
```

```js [Fetch]
await fetch("http://localhost:3000/collections/posts/items/01HPT7B2X", {
  method: "DELETE",
  headers: { Authorization: "SecretApiKey" },
});
```

```sh [cURL]
# DELETE /collections/{collection_name}/items/{id}
#
# Response: 200 OK

curl -X DELETE http://localhost:3000/collections/posts/items/01HPT7B2X \
  -H "Authorization: SecretApiKey"
```

:::

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `DELETE` |
| **Path** | `/collections/{collection}/items/{id}` |

### Path Parameters

| Param | Type | Description |
|---|---|---|
| `collection` | `string` | Collection name |
| `id` | `string` | Item ID to delete (must match the collection's `id_type`) |

**Response:** `200 OK` (no response body)
