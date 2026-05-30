# Import & Export

Aperio provides two HTTP endpoints to create portable snapshots of the entire search index and restore them later. Both use `fjall::Snapshot` internally, so they produce a point-in-time consistent view without blocking concurrent writes.

> **Auth required**: `Authorization: <main_api_key>` header.

## Export

`POST /backup/export`

Creates a snapshot of the entire database and writes it to a file in the configured [dumps folder](#dumps-folder).

```sh
curl -X POST http://localhost:3000/backup/export \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{}'
```

Response:
```json
{"ok": true, "size": 12345, "file": "2026-05-30T13-08-29.aperio"}
```

## Import

`POST /backup/import`

Reads a previously exported snapshot file from the dumps folder and restores all data into the running database. Existing data is **erased first**, so after a successful import the database contains exactly what the snapshot captured and nothing else.

```sh
curl -X POST http://localhost:3000/backup/import \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"name": "2026-05-30T13-08-29.aperio"}'
```

Response:
```json
{"ok": true}
```

## Dumps folder

Snapshots are stored in a **dumps folder** on the server's filesystem. The folder path is configured via the `dumps_folder` option in the [config file](/configuration).

## Use cases

| Goal | How |
|---|---|
| **Backup** before a risky operation | `POST /backup/export` — note the returned filename |
| **Clone** to another machine | Export on source, copy the file from the dumps folder, place it in the destination's dumps folder, import |
| **Restore** after data corruption | `POST /backup/import` with the known-good filename |
