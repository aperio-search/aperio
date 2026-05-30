# Viewing Collection Metadata

Returns metadata about the collection, including the number of indexed documents and unique terms in the inverted index.

## Example

`GET /collections/{collection_name}`

Response: `200 OK`

```json
{
  "name": "posts",
  "id_type": "number",
  "document_count": 42,
  "unique_terms": 318
}
```

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
