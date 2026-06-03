# Configuration

Aperio can be tuned via an optional TOML config file. Set the `CONFIG_FILE` environment variable to point to your config file. If unset or missing, safe defaults are used.

## Example

Place a `config.toml` file and mount it into the container:

```sh
docker run \
  -v $(pwd)/config.toml:/data/config.toml \
  -e DATA_DIR=/data \
  -e CONFIG_FILE=/data/config.toml \
  -p 3000:3000 \
  ghcr.io/aperio-search/aperio
```

Example `config.toml`:

```toml
min_token_length = 3
max_string_shard_size = 1000
max_roaring_shard_size = 100000
index_interval_ms = 900
max_queue_batch_size = 1000

log_level = "info"
main_api_key = "my-secret-key"
search_api_key = "my-search-key"
dumps_folder = "/data/dumps"

# fst_enabled = false    # uncomment to disable FST suggest index
fst_max_words = 250000
fst_max_size_kb = 2048
fst_consolidate_interval_secs = 300
```

## Indexing Behaviour

| Field | Type | Default | Description |
|---|---|---|---|
| `min_token_length` | `integer` | `3` | Minimum length of indexed tokens — shorter tokens are discarded during indexing |
| `max_string_shard_size` | `integer` | `1000` | Max document IDs per string posting-list shard (only applies to `"id_type": "string"`) |
| `max_roaring_shard_size` | `integer` | `100000` | Max document IDs per roaring bitmap shard (only applies to `"id_type": "number"`) |
| `index_interval_ms` | `integer` | `900` | Interval in milliseconds between background index queue flushes. Lower values reduce write-to-search latency; higher values batch more work per flush |
| `max_queue_batch_size` | `integer` | `5000` | Maximum items to pull from the index queue per background tick. Lower values reduce per-tick memory usage during bulk ingestion; higher values drain the queue faster |

## Vocabulary Index (FST)

Aperio maintains a per-collection **Finite State Transducer** (FST) as a vocabulary index of all indexed terms. The FST powers the `GET /collections/{name}/suggest` endpoint and is rebuilt incrementally via periodic consolidation.

| Field | Type | Default | Description |
|---|---|---|---|
| `fst_enabled` | `boolean` | `true` | Set to `false` to disable the FST vocabulary index entirely. Existing `.fst` files are deleted on next startup, the suggest endpoint returns empty results, and there is no indexing overhead. Re-enabling starts with an empty FST and only terms indexed from that point forward are added |
| `fst_max_words` | `integer` | `250000` | Maximum number of unique words per FST. Prevents unbounded growth |
| `fst_max_size_kb` | `integer` | `2048` | Maximum FST file size in kilobytes |
| `fst_consolidate_interval_secs` | `integer` | `300` | Seconds of inactivity before a dirty FST is consolidated to disk |

## Server & Authentication

| Field | Type | Default | Description |
|---|---|---|---|
| `log_level` | `string` | `"info"` | Log level: `"trace"`, `"debug"`, `"info"`, `"warn"`, or `"error"`. Overridden by the `RUST_LOG` environment variable if set |
| `main_api_key` | `string` | `SecretApiKey` | Main API key with full access to all endpoints |
| `search_api_key` | `string` | `PublicApiKey` | Search-only API key for `search` endpoint |
| `dumps_folder` | `string` | *(unset)* | Directory where backup snapshots are written to and read from. If not set, `/backup/export` and `/backup/import` return a `400` error. Must be an absolute or relative path writable by the server process |
