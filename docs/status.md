# Status

Health check endpoint to verify the server is running.

## Example

`GET /status`

Response: `200 OK`

```json
{
  "ok": true
}
```

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `GET` |
| **Path** | `/status` |

### Response Body

| Field | Type | Description |
|---|---|---|
| `ok` | `boolean` | Always `true` when the server is healthy |

**Response:** `200 OK`
