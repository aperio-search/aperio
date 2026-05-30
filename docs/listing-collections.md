# Listing Collections

> **Auth required**: `Authorization: <main_api_key>` header.

Returns a list of all collections with their names and ID types.

## Example

`GET /collections`

Response: `200 OK`

```json
{
  "collections": [
    {
      "name": "posts",
      "id_type": "number"
    },
    {
      "name": "comments",
      "id_type": "string"
    }
  ]
}
```

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
