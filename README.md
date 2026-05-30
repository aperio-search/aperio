<h1 align="center">Aperio</h1>

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

### [Read the docs →](https://github.com/aperio-search/aperio#readme)

## What is Aperio?

Aperio is an screamingly fast, ultra-lean search engine built on top of [fjall](https://github.com/fjall-rs/fjall) and powered by Rust. It's designed as **a lightweight alternative to Elasticsearch** for applications that need ultra-low latency search keeping memory usage minimal even with massive datasets.

## Features

- **Screamingly Fast**: Engineered for performance, delivering ultra-low latency search results.
- **Low RAM Footprint**: Highly resource-efficient, keeping memory usage minimal even with massive datasets.
- **Autocomplete**: Built-in autocomplete endpoint to provide real-time suggestions as users type.
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
│     /collections  /search  /suggest  /items       │
├───────────────────────────────────────────────────┤
│            Store Engine (src/store.rs)            │
│  Inverted Index  ·  Tokenization  ·  ID Strategy  │
├───────────────────────────────────────────────────┤
│            fjall LSM-tree Database                │
│        Keyspaces: _collections, _index_queue,     │
│           {col}.inverted, {col}.docs              │
└───────────────────────────────────────────────────┘
```

The server has three layers:

1. **HTTP Layer** — Axum router exposing REST endpoints.
2. **Store Engine** — Core logic: tokenization, inverted index management, search/insert.
3. **Persistence Layer** — [fjall](https://github.com/fjall-rs/fjall) LSM-tree database for on-disk storage.

### HTTP Layer (`src/routes.rs`)

An Axum `Router` maps endpoints to handler functions that delegate to the `Store`. All state is shared via `Arc<Store>`.

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
| `GET` | `/collections/{name}/suggest?q=...` | Autocomplete |

### Store Engine (`src/store.rs`)

The `Store` struct is the heart of Aperio. It holds:

- **`db: fjall::Database`** — the underlying database handle.
- **`config: StoreConfig`** — tunable parameters (shard sizes, token length, compression, index interval).
- **`collections: RwLock<HashMap<String, IdType>>`** — in-memory registry of known collections and their ID type.
- **`lock: Mutex<()>`** — serializes write operations (upsert/delete) for index consistency.
- **`next_seq: AtomicU64`** — monotonic sequence counter for the indexing queue.
- **`background_active: AtomicBool`** — whether the background indexer is running.

#### Tokenization

Document content is tokenized using [charabia](https://github.com/meilisearch/charabia):

```
content → tokenize() → filter(is_word) → lemma() → filter(min_token_length)
```

Tokens are deduplicated into a `HashSet<String>` before indexing.

#### Inverted Index

Each collection has an inverted index stored in a dedicated fjall keyspace (`{name}.inverted`). For every unique token (word), posting lists map to document IDs.

**Word markers** — an empty key (`word` → empty bytes) signals that a word exists in the index, enabling fast prefix scans for autocomplete.

#### Two ID Strategies

Collections are created with an `id_type` that determines the posting list format:

| `id_type` | Storage format | Data structure |
|---|---|---|
| `string` | rkyv-archived shards | `PostingShard { first, last, ids: Vec<String> }` |
| `number` | Serialized bitmap shards | `RoaringTreemap` per shard |

##### String IDs

Posting lists are split into shards of configurable `max_shard_size` (default 1000). Each shard stores sorted `Vec<String>` archived via rkyv. A binary search across shards locates the correct shard for insertion.

A `Vec<u64>` would be faster for posting-list operations, but `u64` can't represent arbitrary string IDs like `UUIDs`, so `Vec<String>` is used as the general-purpose format.

##### Number IDs

Posting lists use [RoaringTreemap](https://github.com/RoaringBitmap/roaring-rs) bitmaps, sharded at `max_roaring_shard_size` (default 100,000). Bitmaps offer compact storage and fast bitwise intersection for multi-term queries.

#### Search Execution

1. **Tokenize** the query string.
2. **List shard indices** for each token in parallel (via `std::thread::scope`).
3. **Sort tokens by shard count** (rarest-first optimization).
4. **Load posting lists**: for string IDs, merge shards in a sorted iterative merge; for number IDs, union shard bitmaps per word, then compute the intersection.
5. **Apply sort and pagination**: sort by ID ascending or descending, apply optional `after` cursor, cap at `take`.

#### Search: String IDs

For string-ID collections, each shard is an rkyv-archived `PostingShard`. The engine loads all shards for the rarest word, then iterates through its sorted IDs, checking membership in other words' shards via binary search.

#### Search: Number IDs

For number-ID collections, each shard is a `RoaringTreemap`. Per word, all shards are merged with bitwise OR. Words are then intersected with bitwise AND. The resulting bitmap is iterated in ascending or descending order.

### Background Indexing (`spawn_background`)

When the background indexer is active, `upsert()` writes to a FIFO queue (`_index_queue` keyspace) instead of directly updating the index. A `tokio::spawn` task polls the queue at `index_interval` (default 900ms) and calls `process_pending_queue()` to drain entries through `upsert_internal()`.

This batches write operations and reduces lock contention. When the background indexer is not active (e.g., in tests), `upsert()` calls `upsert_internal()` synchronously.

### Persistence Layer (fjall)

[fjall](https://github.com/fjall-rs/fjall) is an embedded LSM-tree storage engine (a RocksDB/Sled alternative). Aperio uses these fjall keyspaces:

| Keyspace | Purpose |
|---|---|
| `_collections` | Collection name → `IdType` mapping |
| `_index_queue` | Pending index operations (background indexing) |
| `{name}.inverted` | Inverted index per collection (word → posting lists) |
| `{name}.docs` | Document tokens per collection (id → `Vec<String>`) |

Configurable fjall options exposed via `StoreConfig`:

- `write_buffer_size` — memtable size.
- `compression` — `"none"` or `"lz4"` for data block compression.
- `block_cache_size` — global block cache for the database.

### Configuration (`src/config.rs`)

Aperio reads an optional TOML config file (`CONFIG_FILE` env var). Parsing is silently lenient and errors fall back to defaults with a warning. The `AppConfig` struct maps one-to-one with `StoreConfig` fields plus server-level options (`block_cache_size`, `maintenance_threads`, `log_level`).

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
    → store.upsert(name, id, content)
      → [background active?]
        → Yes: write to _index_queue → return
        → No:  lock() → upsert_internal()
          → tokenize content (charabia)
          → load old tokens from {name}.docs
          → remove stale posting list entries
          → add/update posting list entries
          → store new tokens in {name}.docs
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
      → return Vec<String>
```

> [!WARNING]
Treat the Architecture section as a **narrative companion** for developers who enjoy reading about low level engineering, not as operational documentation you would rely on for debugging or performance tuning. **If something here contradicts the code, the code wins.**

## About

Aperio is built for those who prioritizes simplicity and performance over features like ranking, filtering, and ordering, trading them for extreme efficiency and high throughput.

## Sponsor

Love using Aperio? Consider becoming a sponsor! As an independent open-source project, we rely on community backing to keep Aperio screamingly fast, ultra-lean, and actively maintained. Take a look at our [GitHub Sponsors](https://github.com/sponsors/andresribeiro) page to see how you can help.
