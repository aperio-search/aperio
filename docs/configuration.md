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
  andresribeiro/aperio
```

Example `config.toml`:

```toml
min_token_length = 2
max_shard_size = 1000
max_roaring_shard_size = 100000
block_cache_size = 536870912       # 512 MiB
write_buffer_size = 67108864       # 64 MiB
maintenance_threads = 4
compression = "lz4"
block_size = 65536                 # 64 KiB
log_level = "info"                 # trace, debug, info, warn, error
index_interval_ms = 900            # ms between index queue flushes
main_api_key = "my-secret-key"     # main API key (full access)
search_api_key = "my-search-key"   # search-only API key (search & suggest only)
dumps_folder = "/data/dumps"       # backup snapshot directory
```

## Configuration Reference

| Field | Type | Default | Description |
|---|---|---|---|
| `min_token_length` | `integer` | `2` | Minimum length of indexed tokens — shorter tokens are discarded during indexing |
| `max_shard_size` | `integer` | `1000` | Max document IDs per string posting-list shard |
| `max_roaring_shard_size` | `integer` | `100000` | Max document IDs per roaring bitmap shard (only applies to `number` collections) |
| `block_cache_size` | `integer` (bytes) | `33554432` (32 MiB) | fjall LSM block cache capacity. Recommended ~20-25% of available memory |
| `write_buffer_size` | `integer` (bytes) | `67108864` (64 MiB, fjall default) | Per-keyspace memtable (write buffer) size. Larger values reduce write amplification at the cost of memory |
| `maintenance_threads` | `integer` | `min(# CPUs, 4)` | Number of background worker threads for compaction, flush, and journal maintenance |
| `compression` | `string` | `"none"` (fjall default) | Data block compression algorithm: `"none"` or `"lz4"` |
| `block_size` | `integer` (bytes) | `4096` (4 KiB, fjall default) | Data block size. Larger values (e.g. 64 KiB) improve range-scan throughput; smaller values reduce read amplification for point lookups |
| `log_level` | `string` | `"info"` | Log level: `"trace"`, `"debug"`, `"info"`, `"warn"`, or `"error"`. Overridden by the `RUST_LOG` environment variable if set |
| `index_interval_ms` | `integer` | `900` | Interval in milliseconds between background index queue flushes. Lower values reduce write-to-search latency; higher values batch more work per flush |
| `main_api_key` | `string` | `SecretApiKey` | Main API key with full access to all endpoints. Overridden by the `MAIN_API_KEY` environment variable if set |
| `search_api_key` | `string` | `PublicApiKey` | Search-only API key for `search` and `suggest` endpoints. Overridden by the `SEARCH_API_KEY` environment variable if set |
| `dumps_folder` | `string` | *(unset)* | Directory where backup snapshots are written to and read from. If not set, `/backup/export` and `/backup/import` return a `400` error. Must be an absolute or relative path writable by the server process |
