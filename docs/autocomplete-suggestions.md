# Autocomplete Suggestions

> **Auth required**: `Authorization: <main_api_key>` or `Authorization: <search_api_key>` header.

Returns autocomplete suggestions based on the last word in the query. For example, searching `"application pro"` will suggest completions for `"pro"` (e.g. `"programming"`, `"process"`).

> Suggestions do **not** consider sentence or phrase context.

## Example

`GET /collections/{collection_name}/suggest?q=app`

Response: `200 OK`

```json
{
  "suggestions": ["apple", "application", "apricot"]
}
```

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
