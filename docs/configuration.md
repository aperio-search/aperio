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
min_token_length = 3
max_shard_size = 1000
max_roaring_shard_size = 100000
block_cache_size = 536870912           # 512 MiB
write_buffer_size = 67108864           # 64 MiB
maintenance_threads = 4
compression = "lz4"
roaring_inverted_block_size = 16384    # 16 KiB — roaring bitmap inverted index keyspace
string_inverted_block_size = 65536     # 64 KiB — string ID inverted index keyspace
docs_block_size = 8192                 # 8 KiB — document storage keyspace
queue_block_size = 32768               # 32 KiB — index queue keyspace
meta_block_size = 8192                 # 8 KiB — collection metadata keyspace
log_level = "info"                     # trace, debug, info, warn, error
index_interval_ms = 900                # ms between index queue flushes
max_queue_batch_size = 1000            # items processed per background tick
main_api_key = "my-secret-key"         # main API key (full access)
search_api_key = "my-search-key"       # search-only API key (search endpoint only)
dumps_folder = "/data/dumps"           # backup snapshot directory
```

## Configuration Reference

| Field | Type | Default | Description |
|---|---|---|---|
| `min_token_length` | `integer` | `3` | Minimum length of indexed tokens — shorter tokens are discarded during indexing |
| `max_shard_size` | `integer` | `1000` | Max document IDs per string posting-list shard |
| `max_roaring_shard_size` | `integer` | `100000` | Max document IDs per roaring bitmap shard (only applies to `number` collections) |
| `block_cache_size` | `integer` (bytes) | `33554432` (32 MiB) | fjall LSM block cache capacity. Recommended ~20-25% of available memory |
| `write_buffer_size` | `integer` (bytes) | `67108864` (64 MiB, fjall default) | Per-keyspace memtable (write buffer) size. Larger values reduce write amplification at the cost of memory |
| `maintenance_threads` | `integer` | `min(# CPUs, 4)` | Number of background worker threads for compaction, flush, and journal maintenance |
| `compression` | `string` | `"none"` (fjall default) | Data block compression algorithm: `"none"` or `"lz4"` |
| `roaring_inverted_block_size` | `integer` (bytes) | `16384` (16 KiB) | Data block size for `{collection}.inverted` keyspaces of **number**-type collections (RoaringTreemap bitmaps). Small blocks favour point lookups during search |
| `string_inverted_block_size` | `integer` (bytes) | `65536` (64 KiB) | Data block size for `{collection}.inverted` keyspaces of **string**-type collections (rkyv-archived shards) |
| `docs_block_size` | `integer` (bytes) | `8192` (8 KiB) | Data block size for `{collection}.docs` keyspaces (stored documents). Larger blocks improve range-scan throughput |
| `queue_block_size` | `integer` (bytes) | `32768` (32 KiB) | Data block size for `_index_queue` keyspace. Larger blocks reduce write amplification for sequential append |
| `meta_block_size` | `integer` (bytes) | `8192` (8 KiB) | Data block size for `_collections` keyspace (system metadata). Small blocks favour point lookups |
| `log_level` | `string` | `"info"` | Log level: `"trace"`, `"debug"`, `"info"`, `"warn"`, or `"error"`. Overridden by the `RUST_LOG` environment variable if set |
| `index_interval_ms` | `integer` | `900` | Interval in milliseconds between background index queue flushes. Lower values reduce write-to-search latency; higher values batch more work per flush |
| `max_queue_batch_size` | `integer` | `1000` | Maximum items to pull from the index queue per background tick. Lower values reduce per-tick memory usage during bulk ingestion; higher values drain the queue faster |
| `main_api_key` | `string` | `SecretApiKey` | Main API key with full access to all endpoints |
| `search_api_key` | `string` | `PublicApiKey` | Search-only API key for `search` endpoint |
| `dumps_folder` | `string` | *(unset)* | Directory where backup snapshots are written to and read from. If not set, `/backup/export` and `/backup/import` return a `400` error. Must be an absolute or relative path writable by the server process |
