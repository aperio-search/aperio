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
max_string_shard_size = 1000
max_roaring_shard_size = 100000
index_interval_ms = 900
max_queue_batch_size = 1000

block_cache_size = 536870912           # 512 MiB
maintenance_threads = 4

inverted_write_buffer_size = 67108864  # 64 MiB
docs_buffer_size = 67108864            # 64 MiB
index_queue_buffer_size = 67108864     # 64 MiB
inverted_hash_ratio = 8.0
docs_hash_ratio = 8.0
inverted_roaring_block_size = 16384    # 16 KiB
inverted_string_block_size = 65536     # 64 KiB
docs_block_size = 8192                 # 8 KiB
queue_block_size = 32768               # 32 KiB
meta_block_size = 8192                 # 8 KiB

log_level = "info"
main_api_key = "my-secret-key"
search_api_key = "my-search-key"
dumps_folder = "/data/dumps"
```

## Indexing Behaviour

| Field | Type | Default | Description |
|---|---|---|---|
| `min_token_length` | `integer` | `3` | Minimum length of indexed tokens — shorter tokens are discarded during indexing |
| `max_string_shard_size` | `integer` | `1000` | Max document IDs per string posting-list shard (only applies to `"id_type": "string"`) |
| `max_roaring_shard_size` | `integer` | `100000` | Max document IDs per roaring bitmap shard (only applies to `"id_type": "number"`) |
| `index_interval_ms` | `integer` | `900` | Interval in milliseconds between background index queue flushes. Lower values reduce write-to-search latency; higher values batch more work per flush |
| `max_queue_batch_size` | `integer` | `5000` | Maximum items to pull from the index queue per background tick. Lower values reduce per-tick memory usage during bulk ingestion; higher values drain the queue faster |

## fjall Engine

Settings that control the underlying fjall LSM-tree storage engine.

| Field | Type | Default | Description |
|---|---|---|---|
| `block_cache_size` | `integer` (bytes) | `33554432` (32 MiB) | Global block cache capacity. Recommended ~20-25% of available memory |
| `maintenance_threads` | `integer` | `min(# CPUs, 4)` | Number of background worker threads for compaction, flush, and journal maintenance |

## Per-Keyspace Tuning

Apply independently to each keyspace — write buffers, compression, block sizes, and hash index ratios. The `_collections` keyspace has a fixed 256 KiB write buffer and is not configurable.

### Compression

Data block compression per keyspace. Options: `"none"` or `"lz4"`. Disabled by default on all keyspaces.

| Field | Type | Default | Applies To |
|---|---|---|---|
| `docs_compression` | `string` | (none) | `{collection}.docs` |
| `inverted_string_compression` | `string` | (none) | `{collection}.inverted` for string-ID collections |
| `inverted_roaring_compression` | `string` | (none) | `{collection}.inverted` for number-ID collections |
| `index_queue_compression` | `string` | (none) | `_index_queue` |
| `collections_compression` | `string` | (none) | `_collections` |

### Write Buffers

Memtable (write buffer) size per keyspace. Larger values reduce write amplification at the cost of memory. When `None`, fjall's built-in default is used.

| Field | Type | Default | Applies To |
|---|---|---|---|
| `inverted_write_buffer_size` | `integer` (bytes) | (fjall default) | `{collection}.inverted` |
| `docs_buffer_size` | `integer` (bytes) | (fjall default) | `{collection}.docs` |
| `index_queue_buffer_size` | `integer` (bytes) | (fjall default) | `_index_queue` |

### Hash Index Ratios

Hash index ratio for prefix bloom filters. Higher values give more buckets per key, improving point-read performance. `0.0` disables the hash index entirely. fjall benchmark sweet spot is `8.0`.

| Field | Type | Default | Applies To |
|---|---|---|---|
| `inverted_hash_ratio` | `float` | `8.0` | `{collection}.inverted` |
| `docs_hash_ratio` | `float` | `8.0` | `{collection}.docs` |

### Data Block Sizes

Data block size per keyspace. Larger blocks improve range-scan throughput; smaller blocks favour point lookups.

| Field | Type | Default | Applies To |
|---|---|---|---|
| `docs_block_size` | `integer` (bytes) | `8192` (8 KiB) | `{collection}.docs` |
| `queue_block_size` | `integer` (bytes) | `32768` (32 KiB) | `_index_queue` |
| `meta_block_size` | `integer` (bytes) | `8192` (8 KiB) | `_collections` |
| `inverted_string_block_size` | `integer` (bytes) | `65536` (64 KiB) | `{collection}.inverted` for string-ID collections (rkyv shards) |
| `inverted_roaring_block_size` | `integer` (bytes) | `16384` (16 KiB) | `{collection}.inverted` for number-ID collections (RoaringTreemap bitmaps) |

## Server & Authentication

| Field | Type | Default | Description |
|---|---|---|---|
| `log_level` | `string` | `"info"` | Log level: `"trace"`, `"debug"`, `"info"`, `"warn"`, or `"error"`. Overridden by the `RUST_LOG` environment variable if set |
| `main_api_key` | `string` | `SecretApiKey` | Main API key with full access to all endpoints |
| `search_api_key` | `string` | `PublicApiKey` | Search-only API key for `search` endpoint |
| `dumps_folder` | `string` | *(unset)* | Directory where backup snapshots are written to and read from. If not set, `/backup/export` and `/backup/import` return a `400` error. Must be an absolute or relative path writable by the server process |


