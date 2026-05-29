# Inserting Items

Inserts or updates an item in the specified collection.

## Example

`POST /collections/{collection_name}/items`

Request Body:

```json
{
  "id": "01HPT7B2X...",
  "content": "Lorem ipsum dolor sit amet, consectetur adipiscing elit..."
}
```

Response: `200 OK`

> [!WARNING]
> For better performance, use sequential IDs (auto-incrementing integers, UUIDv7, ULID...). Using completely random IDs will significantly slow down the writing speed.

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `POST` |
| **Path** | `/collections/{collection}/items` |

### Request Body

| Param | Type | Description |
|---|---|---|
| `id` | `string` | Item ID |
| `content` | `string` | Item content to index |

**Response:** `200 OK` (no response body)
