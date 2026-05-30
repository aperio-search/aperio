# Import & Export

Aperio provides two HTTP endpoints to create portable snapshots of the entire search index and restore them later. Both use `fjall::Snapshot` internally, so they produce a point-in-time consistent view without blocking concurrent writes.

> **Auth required**: `Authorization: <main_api_key>` header.

## Export

`POST /backup/export`

Creates a snapshot of the entire database and writes it to a file on the server's filesystem. The file contains all collections, documents, and index data in a portable binary format.

```sh
curl -X POST http://localhost:3000/backup/export \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"path": "/tmp/aperio-snapshot.bin"}'
```

Response:
```json
{"ok": true, "size": 12345, "path": "/tmp/aperio-snapshot.bin"}
```

## Import

`POST /backup/import`

Reads a previously exported snapshot file from the server's filesystem and restores all data into the running database. Existing data is **merged** — keys from the snapshot overwrite matching keys in the database.

```sh
curl -X POST http://localhost:3000/backup/import \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"path": "/tmp/aperio-snapshot.bin"}'
```

Response:
```json
{"ok": true}
```

## Use cases

| Goal | Command |
|---|---|
| **Backup** before a risky operation | `POST /backup/export` to a safe location |
| **Clone** to another machine | Export on source, copy the file, import on destination |
| **Restore** after data corruption | `POST /backup/import` from a known-good snapshot |
