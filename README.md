<h1 align="center">Aperio</h1>

<p align="center">
<img src="assets/banner.jpg" alt="Aperio banner" width="100%">
</p>

<p align="center">
<a href="https://github.com/aperio-search/aperio"><img src="https://img.shields.io/badge/aperio-Screamingly%20fast-green" alt="Aperio" height=50></a>
<img src="https://img.shields.io/github/stars/aperio-search/aperio" alt="stars">
<img src="https://img.shields.io/badge/language-Rust-orange" alt="Rust">
</p>

<div align="center">
  <a href="https://aperiosearch.com/quickstart.html">Quickstart</a>
  <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
  <a href="https://aperiosearch.com/about.html">About</a>
  <br />
</div>

### [Read the docs →](https://aperiosearch.com)

## What is Aperio?

Aperio is an screamingly fast, ultra-lean search engine built on top of [fjall](https://github.com/fjall-rs/fjall) and powered by Rust. It's designed as **a lightweight alternative to Elasticsearch** for applications that need ultra-low latency search keeping memory usage minimal even with massive datasets.

## Features

- **Screamingly Fast**: Engineered for performance, delivering ultra-low latency search results.
- **Low RAM Footprint**: Highly resource-efficient, keeping memory usage minimal even with massive datasets.
- **Full Unicode Support**: Built-in normalization and encoding compatibility to handle global data flawlessly.
- **DevOps-Free**: Easy to deploy, configure, and maintain without needing dedicated DevOps expertise.

## Install

Aperio runs on Linux (x64 & arm64) and macOS (x64 & Apple Silicon).

### Docker

```bash
docker build -t aperio .
docker run --rm -p 3000:3000 -v "$(pwd)/data:/data" aperio
```

## Architecture

### System Overview

```
┌───────────────────────────────────────────────────┐
│                    Client                         │
├───────────────────────────────────────────────────┤
│                   HTTP (port 3000)                │
├───────────────────────────────────────────────────┤
│          Axum Router (src/routes.rs)              │
│     /collections  /search  /items                 │
├───────────────────────────────────────────────────┤
│            Store Engine (src/store/)              │
│  Inverted Index  ·  Tokenization  ·  ID Strategy  │
├───────────────────────────────────────────────────┤
│    Auth (src/auth.rs)  ·  Backup (src/backup.rs)  │
├───────────────────────────────────────────────────┤
│            fjall LSM-tree Database                │
│        Keyspaces: _collections, _index_queue,     │
│           {col}.inverted, {col}.docs              │
└───────────────────────────────────────────────────┘
```

The server has four layers:

1. **HTTP Layer** — Axum router exposing REST endpoints.
2. **Auth & Backup** — API key authentication (`src/auth.rs`) and snapshot export/import (`src/backup.rs`).
3. **Store Engine** — Core logic: tokenization, inverted index management, search/insert (`src/store/` sub-modules).
4. **Persistence Layer** — [fjall](https://github.com/fjall-rs/fjall) LSM-tree database for on-disk storage.

### HTTP Layer (`src/routes.rs`)

An Axum `Router` maps endpoints to handler functions that delegate to the `Store`. All state is shared via `Arc<AppState>` (`store` + optional `dumps_folder`). Every endpoint (except `/status`) requires an API key via the `Authorization` header, enforced by `src/auth.rs`. Two tiers of access: the **main** key has full access; the **search** key is restricted to `GET …/search`.

| Method | Path | Handler |
|---|---|---|
| `GET` | `/status` | Health check |
| `GET` | `/collections` | List collections |
| `POST` | `/collections` | Create collection |
| `GET` | `/collections/{name}` | Collection metadata |
| `DELETE` | `/collections/{name}` | Delete collection |
| `POST` | `/collections/{name}/items` | Upsert document |
| `DELETE` | `/collections/{name}/items/{id}` | Delete document |
| `GET` | `/collections/{name}/search?q=...` | Search documents |
| `POST` | `/backup/export` | Export database snapshot to a file |
| `POST` | `/backup/import` | Import a snapshot from the dumps folder |

### Store Engine (`src/store/`)

The `Store` struct (in `src/store/mod.rs`) is the heart of Aperio. It holds:

- **`db: fjall::Database`** — the underlying database handle.
- **`config: StoreConfig`** — tunable parameters (shard sizes, token length, compression, index interval).
- **`collections: RwLock<HashMap<String, CollectionMeta>>`** — in-memory registry of known collections, their ID type and searchable fields.
- **`lock: Mutex<()>`** — serializes write operations (upsert/delete) for index consistency.
- **`next_seq: AtomicU64`** — monotonic sequence counter for the indexing queue.
- **`background_active: AtomicBool`** — whether the background indexer is running.

The store logic is split across sub-modules:
- `src/store/config.rs` — `StoreConfig`, `IdType`, `CollectionMeta`, `PostingShard`, `QueuedIndex`.
- `src/store/posting_list.rs` — shard-based posting list operations for both ID strategies.
- `src/store/search.rs` — search execution (intersection, cursor pagination) for string and number IDs.

#### Tokenization

Document content is tokenized using [charabia](https://github.com/meilisearch/charabia):

```
content → tokenize() → filter(is_word) → lemma() → filter(min_token_length)
```

Tokens are deduplicated into a `HashSet<String>` before indexing.

#### Inverted Index

Each collection has an inverted index stored in a dedicated fjall keyspace (`{name}.inverted`). For every unique token (word), posting lists map to document IDs.

#### Two ID Strategies

Collections are created with an `id_type` that determines the posting list format:

| `id_type` | Storage format | Data structure |
|---|---|---|
| `string` | rkyv-archived shards | `PostingShard { first, last, ids: Vec<String> }` |
| `number` | Serialized bitmap shards | `RoaringTreemap` per shard |

##### String IDs

Posting lists are split into shards of configurable `max_string_shard_size` (default 1000). Each shard stores sorted `Vec<String>` archived via rkyv. A binary search across shards locates the correct shard for insertion.

A `Vec<u64>` would be faster for posting-list operations, but `u64` can't represent arbitrary string IDs like `UUIDs`, so `Vec<String>` is used as the general-purpose format.

##### Number IDs

Posting lists use [RoaringTreemap](https://github.com/RoaringBitmap/roaring-rs) bitmaps, sharded at `max_roaring_shard_size` (default 100,000). Bitmaps offer compact storage and fast bitwise intersection for multi-term queries.

#### Search Execution

1. **Tokenize** the query string.
2. **List shard indices** for each token in parallel (via rayon).
3. **Sort tokens by shard count** (rarest-first optimization).
4. **Load posting lists**: for string IDs, merge shards in a sorted iterative merge; for number IDs, union shard bitmaps per word, then compute the intersection.
5. **Apply sort and pagination**: sort by ID ascending or descending, apply optional `after` cursor, cap at `take`.

#### Search: String IDs

For string-ID collections, each shard is an rkyv-archived `PostingShard`. The engine loads all shards for the rarest word, then iterates through its sorted IDs, checking membership in other words' shards via binary search.

#### Search: Number IDs

For number-ID collections, each shard is a `RoaringTreemap`. Per word, all shards are merged with bitwise OR. Words are then intersected with bitwise AND. The resulting bitmap is iterated in ascending or descending order.

### Background Indexing (`spawn_background`)

When the background indexer is active, `upsert()` writes to a FIFO queue (`_index_queue` keyspace) instead of directly updating the index. A `tokio::spawn` task polls the queue at `index_interval` (default 900ms) and dispatches `process_pending_queue()` on Tokio's blocking thread pool via `spawn_blocking`. Within each batch, tokenization runs in parallel across queued items using rayon, then posting list mutations are applied sequentially to a shared `OwnedWriteBatch`.

This batches write operations and reduces lock contention. When the background indexer is not active (e.g., in tests), `upsert()` calls `upsert_internal()` synchronously.

### Persistence Layer (fjall)

[fjall](https://github.com/fjall-rs/fjall) is an embedded LSM-tree storage engine (a RocksDB/Sled alternative). Aperio uses these fjall keyspaces:

| Keyspace | Purpose |
|---|---|
| `_collections` | Collection name → `CollectionMeta` (ID type + searchable fields) |
| `_index_queue` | Pending index operations (background indexing) |
| `{name}.inverted` | Inverted index per collection (word → posting lists) |
| `{name}.docs` | Full JSON documents per collection (id → JSON bytes) |

Configurable fjall options exposed via `StoreConfig`:

- `inverted_write_buffer_size` — memtable size for `{collection}.inverted`.
- `docs_buffer_size` — memtable size for `{collection}.docs`.
- `index_queue_buffer_size` — memtable size for `_index_queue`.
- `docs_compression`, `inverted_string_compression`, `inverted_roaring_compression`, `index_queue_compression`, `collections_compression` — per-keyspace data block compression (`"none"` or `"lz4"`).
- `block_cache_size` — global block cache for the database (set on `Database::builder`, not `StoreConfig`).
- `inverted_roaring_block_size`, `inverted_string_block_size`, `docs_block_size`, `queue_block_size`, `meta_block_size` — per-keyspace data block sizes.
- `inverted_string_hash_ratio`, `inverted_roaring_hash_ratio`, `docs_hash_ratio` — hash index ratios for inverted/doc keyspaces.
- `index_interval` — interval between background index queue flushes.
- `max_queue_batch_size` — items processed per background tick.

### Configuration (`src/config.rs`)

Aperio reads an optional TOML config file (`CONFIG_FILE` env var). Parsing is **strict**: on any read or parse error the process panics with a clear message. The `AppConfig` struct maps one-to-one with `StoreConfig` fields plus server-level options (`block_cache_size`, `maintenance_threads`, `log_level`, `main_api_key`, `search_api_key`, `dumps_folder`).

### Error Handling (`src/error.rs`)

All operations return `Result<T, AppError>`, an enum that maps to appropriate HTTP status codes:

| Error variant | HTTP status |
|---|---|
| `NotFound` | 404 |
| `BadRequest` | 400 |
| `Internal` | 500 |

Axum's `IntoResponse` impl renders errors as JSON: `{"error": "message"}`.

### Data Flow: Document Insertion

```
Client → POST /collections/{name}/items
  → routes::upsert_item()
    → store.upsert(name, doc)
      → [background active?]
        → Yes: write to _index_queue → return
        → No:  lock() → upsert_internal()
          → extract `id` from JSON doc
          → extract searchable field values from JSON doc
          → tokenize combined searchable content (charabia)
          → load old JSON from {name}.docs
          → compute old tokens from old searchable fields
          → remove stale posting list entries
          → add/update posting list entries
          → store full JSON doc in {name}.docs
          → unlock()
```

### Data Flow: Search

```
Client → GET /collections/{name}/search?q=...
  → routes::search()
    → store.search(name, query, sort, take, after)
      → validate collection exists
      → tokenize query
      → parallel: list shard indices per word
      → sort by rarest word first
      → parallel: load posting lists
      → [string IDs]: sorted merge + membership check
      → [number IDs]: bitmap union + intersection
      → apply after-cursor, sort, limit
      → look up full JSON docs from {name}.docs
      → return Vec<serde_json::Value>
```

> [!WARNING]
Treat the Architecture section as a **narrative companion** for developers who enjoy reading about low level engineering, not as operational documentation you would rely on for debugging or performance tuning. **If something here contradicts the code, the code wins.**

## Sponsor

If you find Aperio useful, please consider becoming a sponsor: as an independent open-source project, we rely on community backing to keep Aperio screamingly fast, ultra-lean, and actively maintained. Take a look at our [GitHub Sponsors](https://github.com/sponsors/andresribeiro) page to see how you can help.

If you have a few seconds, a star on GitHub helps us a lot!
