# API Reference

Complete list of all Aperio HTTP endpoints.

> **Authentication**: All endpoints require an `Authorization: <key>` header, except `GET /status` which is public. Use the **main API key** for full access, or the **search API key** for `search` endpoint only. See [Environment Variables](/environment-variables) for configuration.

## Status

| Method | Path | Description |
|---|---|---|
| `GET` | `/status` | Health check (public, no auth required) |

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
| `POST` | `/collections/{collection}/items/bulk` | [Bulk insert or update multiple items](/bulk-ingestion) |
| `DELETE` | `/collections/{collection}/items/{id}` | Delete an item by ID |

## Search

| Method | Path | Description |
|---|---|---|
| `GET` | `/collections/{collection}/search` | Search documents matching a query |
| `GET` | `/collections/{collection}/suggest` | Suggest indexed terms matching a prefix |

## Queue

| Method | Path | Description |
|---|---|---|
| `GET` | `/queue` | Returns the number of documents waiting to be indexed |

## Backup

| Method | Path | Description |
|---|---|---|
| `POST` | `/backup/export` | Export a snapshot of the entire database to a file |
| `POST` | `/backup/import` | Import a snapshot from a file into the database |

See the [Import & Export](/import-export) guide for details and examples.
