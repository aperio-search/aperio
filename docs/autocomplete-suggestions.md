# Autocomplete Suggestions

> **Auth required**: `Authorization: <main_api_key>` or `Authorization: <search_api_key>` header.

Returns autocomplete suggestions based on the last word in the query. For example, searching `"application pro"` will suggest completions for `"pro"` (e.g. `"programming"`, `"process"`).

> Suggestions do **not** consider sentence or phrase context.

## Example

::: code-group

```js [Node.js]
const { suggestions } = await client.suggest("posts", { q: "app" });
// ["apple", "application", "apricot"]
```

```js [Fetch]
const res = await fetch("http://localhost:3000/collections/posts/suggest?q=app", {
  headers: { Authorization: "SecretApiKey" },
});
const { suggestions } = await res.json();
```

```sh [cURL]
# GET /collections/{collection_name}/suggest?q=app
#
# Response: 200 OK
# {
#   "suggestions": ["apple", "application", "apricot"]
# }

curl "http://localhost:3000/collections/posts/suggest?q=app" \
  -H "Authorization: SecretApiKey"
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
| `q` | `string` | — | Word prefix to match (uses the last word if multiple) |

### Response Body

| Field | Type | Description |
|---|---|---|
| `suggestions` | `array` of `string` | Autocomplete suggestions |

**Response:** `200 OK`
