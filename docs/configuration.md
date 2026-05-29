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
