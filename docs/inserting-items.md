# Inserting Items

> **Auth required**: `Authorization: <main_api_key>` header.

Inserts or updates an item in the specified collection. The request body must be a JSON object with an `id` field matching the collection's `id_type`. Only fields listed in the collection's `searchable_fields` are indexed; all other fields are stored but ignored by the search index.

## Example

`POST /collections/{collection_name}/items`

Request Body:

```json
{
  "id": "01HPT7B2X...",
  "title": "Hello World",
  "body": "Lorem ipsum dolor sit amet..."
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

The request body is an arbitrary JSON object. It **must** contain an `id` field matching the collection's `id_type`. All other fields are stored as-is. Only fields listed in the collection's `searchable_fields` param are tokenized and indexed for search.

| Field | Type | Description |
|---|---|---|
| `id` | `string` or `number` | Item ID (must match the collection's `id_type`) |
| `…` | any | Any other JSON fields; only searchable fields are indexed |

**Response:** `200 OK` (no response body)
