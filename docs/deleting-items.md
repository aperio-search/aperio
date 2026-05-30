# Deleting Items

Deletes a single item from a collection by its `ID`.

## Example

`DELETE /collections/{collection_name}/items/{id}`

Response: `200 OK`

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
