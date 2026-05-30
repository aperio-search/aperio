# API Reference

Complete list of all Aperio HTTP endpoints.

## Status

| Method | Path | Description |
|---|---|---|
| `GET` | `/status` | Health check |

## Collections

| Method | Path | Description |
|---|---|---|
| `POST` | `/collections` | Create a new collection |
| `GET` | `/collections` | List all collections |
| `GET` | `/collections/{collection}` | Get collection metadata |
| `DELETE` | `/collections/{collection}` | Delete a collection and all its data |

## Items

| Method | Path | Description |
|---|---|---|
| `POST` | `/collections/{collection}/items` | Insert or update an item |
| `DELETE` | `/collections/{collection}/items/{id}` | Delete an item by ID |

## Search

| Method | Path | Description |
|---|---|---|
| `GET` | `/collections/{collection}/search` | Search documents matching a query |
| `GET` | `/collections/{collection}/suggest` | Autocomplete suggestions for a prefix |

See the [Guides](/quickstart) section for detailed endpoint documentation with parameters and examples.
