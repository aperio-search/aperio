# Creating Collections

A `Collection` must be created before you can insert any data. During creation, you must specify whether your record `IDs` will be `number` or `string`. Because `ID` selection directly impacts database indexing, please consider the following performance guidelines:

- **Use sequential IDs**: Whether you choose `number` or `string`, keeping them sequential is critical. Completely random `IDs` (such as `UUIDv4`) will drastically degrade write performance.
- **Opt for `number` for maximum speed**: For the absolute highest throughput, lowest latency and best storage utilization, use numeric IDs.

> Filtering is not supported directly. However, you can easily bypass this by structuring your collections. For example, if you want to search messages by a specific user, you can simply create a dedicated collection named `messages:[userId]`.

## Example

`POST /collections`

Request Body:

```json
{
  "name": "messages",
  "id_type": "number"
}
```

Response: `201 { "name": "…", "id_type": "…" }`

## Endpoint Definition

| Field | Value |
|---|---|
| **Method** | `POST` |
| **Path** | `/collections` |

### Request Body

| Param | Type | Description |
|---|---|---|
| `name` | `string` | Collection name |
| `id_type` | `string` | `"number"` or `"string"` |

### Response Body

| Field | Type | Description |
|---|---|---|
| `name` | `string` | Collection name |
| `id_type` | `string` | `"number"` or `"string"` |

**Response:** `201 Created`
