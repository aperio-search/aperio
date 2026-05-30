# Error Handling

All API errors return a JSON body with an `error` field describing the problem.

## Response Format

```json
{
  "error": "description of the error"
}
```

## Status Codes

| Code | Meaning | When it occurs |
|---|---|---|
| `400 Bad Request` | Malformed request body or invalid parameters | JSON parse failure, missing required fields, invalid query parameter values |
| `404 Not Found` | The requested resource does not exist | Unknown collection name, unknown item ID, unmatched route |
| `500 Internal Server Error` | Unexpected server-side failure | Storage errors, internal bugs, resource exhaustion |

## Examples

### Collection not found

```
GET /collections/unknown/search?q=test
```

```json
{
  "error": "collection `unknown` not found"
}
```

### Invalid request body

```
POST /collections
Content-Type: application/json

{"name": "mycol"}
```

```json
{
  "error": "missing field `id_type`"
}
```

### Unmatched route

```
GET /nonexistent
```

```json
{
  "error": "not found"
}
```
