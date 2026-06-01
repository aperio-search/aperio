# Queue

Returns the number of documents currently waiting to be indexed.

When documents are inserted, they are queued and indexed asynchronously by a background thread. This endpoint lets you monitor the queue depth.

## Example

`GET /queue`

Response: `200 OK`

```json
{
  "pending": 5
}
```

A value of `0` means the queue is empty and all documents have been indexed.

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `GET` |
| **Path** | `/queue` |
| **Auth** | Main API key required |

### Response Body

| Field | Type | Description |
|---|---|---|
| `pending` | `number` | Number of documents waiting to be processed |

**Response:** `200 OK`
