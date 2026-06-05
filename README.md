<h1 align="center">Aperio</h1>

<p align="center">
<img src="assets/banner.jpg" alt="Aperio banner" width="100%">
</p>

<p align="center">
  <a href="https://github.com/aperio-search/aperio">
    <img src="https://img.shields.io/badge/aperio-Screamingly%20fast-2ea44f?style=for-the-badge" alt="Aperio">
  </a>

  <img src="https://img.shields.io/github/actions/workflow/status/aperio-search/aperio/rust.yml?branch=main&label=Rust%20CI&style=for-the-badge&logo=github&logoColor=white&color=2ea44f" alt="Rust CI Status">

  <a href="https://deps.rs/repo/github/aperio-search/aperio">
    <img src="https://img.shields.io/badge/dependencies-up%20to%20date-2ea44f?style=for-the-badge&logo=rust&logoColor=white" alt="Dependency Status">
  </a>

  <img src="https://img.shields.io/github/stars/aperio-search/aperio?style=for-the-badge&logo=github&logoColor=white&color=dfb317" alt="stars">

  <img src="https://img.shields.io/badge/language-Rust-e4371b?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
</p>

<div align="center">
  <a href="https://aperiosearch.com/quickstart.html">Quickstart</a>
  <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
  <a href="https://aperiosearch.com/about.html">About</a>
  <span>&nbsp;&nbsp;•&nbsp;&nbsp;</span>
  <a href="https://github.com/aperio-search/aperio/blob/main/benchmarks/BENCHMARKING.md">Benchmarks</a>
  <br />
</div>

### [Aperio can search across 900GBs of data in < 1ms using less then 256MB of RAM. Check benchmarks →](/Benchmarks.md)
### [Read the docs →](https://aperiosearch.com)

## What is Aperio?

Aperio is an screamingly fast, ultra-lightweight search engine built on top of [LMDB](https://www.symas.com/lmdb) (via [`heed`](https://github.com/Kerollmops/heed)) and powered by Rust. It's designed as **a lightweight alternative to Elasticsearch** for applications that need ultra-low latency search keeping memory usage minimal even with massive datasets.

## Features

- **Screamingly Fast**: Engineered for performance, delivering ultra-low latency search results.
- **Low RAM Footprint**: Highly resource-efficient, keeping memory usage minimal.
- **Full Unicode Support**: Built-in normalization and encoding compatibility to handle global data flawlessly.
- **DevOps-Free**: Easy to deploy, configure, and maintain without needing dedicated DevOps expertise.

## Install

Aperio runs on Linux (x64 & arm64) and macOS (x64 & Apple Silicon).

### Docker

```bash
docker pull ghcr.io/aperio-search/aperio
docker run --rm -p 3000:3000 -v "$(pwd)/data:/data" ghcr.io/aperio-search/aperio
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
│            Store Engine (src/store/)              │
│  Inverted Index  ·  FST Vocabulary  ·  ID Strategy│
├───────────────────────────────────────────────────┤
│    Auth (src/auth.rs)  ·  Backup (src/backup.rs)  │
├───────────────────────────────────────────────────┤
│               LMDB Database (heed)                │
│        Databases: meta, queue, docs, inverted     │
│      (collection-prefixed keys, e.g. "col\0key")  │
└───────────────────────────────────────────────────┘
```

The server has four layers:

1. **HTTP Layer** — Axum router exposing REST endpoints.
2. **Auth & Backup** — API key authentication (`src/auth.rs`) and snapshot export/import (`src/backup.rs`).
3. **Store Engine** — Core logic: tokenization, inverted index, FST vocabulary index, search/insert (`src/store/` sub-modules).
4. **Persistence Layer** — [LMDB](https://www.symas.com/lmdb) memory-mapped database for on-disk storage.

### HTTP Layer (`src/routes.rs`)

An Axum `Router` maps endpoints to handler functions that delegate to the `Store`. All state is shared via `Arc<AppState>` (`store` + optional `dumps_folder`). Every endpoint (except `/status`) requires an API key via the `Authorization` header, enforced by `src/auth.rs`. Two tiers of access: the **main** key has full access; the **search** key is restricted to `GET …/search`.

| Method | Path | Handler |
|---|---|---|
| `GET` | `/status` | Health check |
| `GET` | `/collections` | List collections |
| `POST` | `/collections` | Create collection |
| `GET` | `/collections/{collection}` | Collection metadata |
| `DELETE` | `/collections/{collection}` | Delete collection |
| `POST` | `/collections/{collection}/items` | Upsert document |
| `DELETE` | `/collections/{collection}/items/{id}` | Delete document |
| `GET` | `/collections/{collection}/search?q=...` | Search documents |
| `GET` | `/collections/{collection}/suggest?q=...` | Suggest indexed terms matching a prefix |
| `POST` | `/backup/export` | Export database snapshot to a file |
| `POST` | `/backup/import` | Import a snapshot from the dumps folder |
| `GET` | `/queue` | Pending index queue depth |

### Store Engine (`src/store/`)

The `Store` struct (in `src/store/mod.rs`) is the heart of Aperio. It holds:

- **`env: heed::Env`** — the underlying LMDB environment handle.
- **`db_meta: DbBytes`** — LMDB database handle for collection metadata (`meta`).
- **`db_queue: DbBytes`** — LMDB database handle for the index queue (`queue`).
- **`db_docs: DbBytes`** — LMDB database handle for stored JSON documents (`docs`).
- **`db_inverted: DbBytes`** — LMDB database handle for the inverted index (`inverted`).
- **`config: StoreConfig`** — tunable parameters (shard sizes, token length, index interval).
- **`collections: RwLock<HashMap<String, CollectionMeta>>`** — in-memory registry of known collections, their ID type and searchable fields.
- **`lock: Mutex<()>`** — serializes write operations (delete/collection mutation) for index consistency.
- **`next_seq: AtomicU64`** — monotonic sequence counter for the indexing queue.

The store logic is split across sub-modules:
- `src/store/config.rs` — `StoreConfig`, `FSTConfig`, `IdType`, `CollectionMeta`, `PostingShard`, `QueuedIndex`.
- `src/store/fst.rs` — per-collection FST vocabulary index (push/pop, consolidation, prefix/fuzzy search).
- `src/store/posting_list.rs` — shard-based posting list operations for both ID strategies.
- `src/store/search.rs` — search execution (intersection, cursor pagination) for string and number IDs.

#### Tokenization

Document content is tokenized using [charabia](https://github.com/meilisearch/charabia):

```
content → tokenize() → filter(is_word) → lemma() → filter(min_token_length)
```

Tokens are deduplicated into a `HashSet<String>` before indexing.

#### Inverted Index

All collections share a single LMDB database (`inverted`) for the inverted index. Keys are prefixed with the collection name and a null byte (e.g. `"mycol\0hello\0000042"` for shard 42 of word "hello" in collection "mycol").

#### Two ID Strategies

Collections are created with an `id_type` that determines the posting list format:

| `id_type` | Storage format | Data structure |
|---|---|---|
| `string` | rkyv-archived shards | `PostingShard { ids: Vec<String> }` |
| `number` | Serialized bitmap shards | `RoaringTreemap` per shard |

##### String IDs

Posting lists are split into shards of configurable `max_string_shard_size` (default 1000). Each shard stores sorted `Vec<String>` archived via rkyv. A binary search across shards locates the correct shard for insertion.

A `Vec<u64>` would be faster for posting-list operations, but `u64` can't represent arbitrary string IDs like `UUIDs`, so `Vec<String>` is used as the general-purpose format.

##### Number IDs

Posting lists use [RoaringTreemap](https://github.com/RoaringBitmap/roaring-rs) bitmaps, sharded at `max_roaring_shard_size` (default 100,000). Bitmaps offer compact storage and fast bitwise intersection for multi-term queries.

#### FST Vocabulary Index

Each collection has an on-disk FST that stores all indexed terms, enabling **term suggestion** via `GET /collections/{name}/suggest?q=...`. Terms are batched during indexing and periodically consolidated to disk. Suggestion uses `StartsWith` prefix matching from the `fst` crate.

### Search Execution

1. **Tokenize** the query string.
2. **List shard indices** for each token (rarest-first optimization).
3. **Load posting lists**: for string IDs, merge shards in a sorted iterative merge; for number IDs, union shard bitmaps per word, then compute the intersection.
4. **Apply sort and pagination**: sort by ID ascending or descending, apply optional `after` cursor, cap at `take`.

#### Search: String IDs

For string-ID collections, each shard is an rkyv-archived `PostingShard`. The engine loads all shards for the rarest word, then iterates through its sorted IDs, checking membership in other words' shards via binary search.

#### Search: Number IDs

For number-ID collections, each shard is a `RoaringTreemap`. Per word, all shards are merged with bitwise OR. Words are then intersected with bitwise AND. The resulting bitmap is iterated in ascending or descending order.

### Background Indexing (`spawn_background`)

`upsert()` always writes to a FIFO queue (`queue` database) and returns immediately. A `tokio::spawn` task polls the queue at `index_interval` (default 900ms) and dispatches `process_pending_queue()` on Tokio's blocking thread pool via `spawn_blocking`. Within each batch, items are processed sequentially: tokenization via charabia, stale token removal, posting list updates, and doc storage — all within a single LMDB write transaction.

This batches write operations and reduces lock contention. In tests, `flush()` can be called to drain the queue synchronously.

### Persistence Layer (LMDB)

[LMDB](https://www.symas.com/lmdb) is an embedded memory-mapped database (a key-value store). Aperio uses these LMDB databases via `heed`:

| Database | Purpose |
|---|---|---|
| `meta` | Collection name → `CollectionMeta` (ID type + searchable fields) |
| `queue` | Pending index operations (background indexing), keyed by sequence number |
| `docs` | Full JSON documents, keyed by `{collection}\0{doc_id}` |
| `inverted` | Inverted index, keyed by `{collection}\0{word}\0{shard_index}` |

Configurable queue options exposed via `StoreConfig`:

- `index_interval` — interval between background index queue flushes.
- `max_queue_batch_size` — items processed per background tick.

### Configuration (`src/config.rs`)

Aperio reads an optional TOML config file (`CONFIG_FILE` env var). Parsing is **strict**: on any read or parse error the process panics with a clear message. The `AppConfig` struct mirrors `StoreConfig` fields (with `index_interval_ms` converted to a `Duration`) plus server-level options (`log_level`, `main_api_key`, `search_api_key`, `dumps_folder`).

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
Client → POST /collections/{collection}/items
  → routes::upsert_item()
    → store.upsert(collection, doc)
      → validate collection exists
      → extract `id` from JSON doc
      → allocate sequence number
      → serialize doc to JSON bytes
      → write QueuedIndex entry to `queue` database
      → return immediately

  (background ticker)
    → store.process_pending_queue()
      → read up to max_queue_batch_size entries from `queue`
      → for each entry:
        → deserialize queued document
        → extract searchable content from JSON
        → tokenize (charabia)
        → load old doc from `docs` database
        → compute old tokens for diff
        → remove stale posting list entries from `inverted`
        → store new doc in `docs` database
        → add new posting list entries to `inverted`
        → delete queue entry
      → commit single LMDB write transaction
```

### Data Flow: Search

```
Client → GET /collections/{collection}/search?q=...
  → routes::search()
    → store.search(collection, query, sort, take, after)
      → validate collection exists
      → tokenize query
      → list shard indices per word, sort by rarest first
      → load posting lists (sequential)
      → [string IDs]: sorted merge + membership check
      → [number IDs]: bitmap union + intersection
      → apply after-cursor, sort, limit
      → look up full JSON docs from `docs` database
      → return Vec<serde_json::Value>
```

> [!WARNING]
Treat the Architecture section as a **narrative companion** for developers who enjoy reading about low level engineering, not as operational documentation you would rely on for debugging or performance tuning. **If something here contradicts the code, the code wins.**

## Sponsor

If you find Aperio useful, please consider becoming a sponsor: as an independent open-source project, we rely on community backing to keep Aperio screamingly fast, ultra-lightweight, and actively maintained. Take a look at our [GitHub Sponsors](https://github.com/sponsors/andresribeiro) page to see how you can help.

If you have a few seconds, a star on GitHub helps us a lot!
