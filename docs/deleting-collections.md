# Delete Collections

Drops an entire collection index and its associated internal storage completely.

## Example

`DELETE /collections/{collection_name}`

Response: `200 OK`

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
