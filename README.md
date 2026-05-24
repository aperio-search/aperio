# Aster

[![License](https://img.shields.io/github/license/yourusername/aster)](LICENSE)
[![Rust](https://img.shields.io/badge/language-Rust-orange.svg)](https://www.rust-lang.org/)

An extremely lightweight search engine, heavy optimized for SSDs, built on Rust and Sled.

Aster organizes search terms into isolated collections (e.g., `posts` for a blog, or `user_messages:1` for specific user data). It is designed for high-throughput, simple full-text search indexing without the overhead of heavy, external search clusters.

- **Lightweight:** Minimal memory footprint, running entirely embedded within your application environment.
- **No Complex Querying:** No filtering, complex aggregations, or heavy boolean logic—just pure, blazing-fast term matching.
- **Sorted Out-of-the-Box:** Results are sorted by document ID (lexicographically). Defaulting to `DESC`, with optional `ASC` retrieval.
- **Disk-Backed Storage:** Leveraging `sled` for zero-copy, concurrent, thread-safe transactional key-value storage.

> Aster is highly optimized for SSDs as it stores all data on disk. Running Aster on a traditional HDD will result in severely degraded search performance.

## Table of Contents

- [Architecture](#architecture)
- [Installation](#installation)
- [Usage](#usage)
- [Endpoints](#endpoints)
- [Performance Optimization](#performance-optimization)
- [Local Development](#local-development)
- [License](#license)

## Architecture

Aster utilizes a two-layer key-value layout inside `sled` to handle inversion and retrieval:

1. **Inverted Index Store:** Maps individual tokens/words to an ascending list of document IDs (`word -> [id1, id2, ...]`).
2. **Internal Document Store:** Maps the original document ID directly to its raw content (`id -> content`), used for quick cleanups and deletions.

When an item is deleted, Aster fetches the document content, extracts its indexed words, purges the ID from the Inverted Index, and finally drops the item from the Internal Document Store.

## Installation

```shell
# Build and run the binary
cargo run --release

```

## Usage

Collections allow you to partition data logically. You can name collections statically (`posts`) or dynamically per entity (`user_messages:123`).

### Document ID Recommendation

Because Aster sorts results by your provided document IDs, using **lexicographically sortable IDs** (such as `ULID`, `UUIDv7`, or zero-padded integers like `000001`) is highly recommended to ensure predictable `ASC`/`DESC` ordering and maximum performance.

---

## Endpoints

### 1. Upsert Item

`POST /collections/:collection/items`

Inserts or updates an item in the specified collection.

**Request Body:**

```json
{
  "id": "01HPT7B2X...",
  "content": "Rust and sled make a powerful, lightweight combination for embedded databases."
}

```

**Response:** `200 OK`

### 2. Search Collection

`GET /collections/:collection/search?q=query_term&sort=desc&take=20&after=`

Multi-word queries perform an **AND** search — only documents matching all terms are returned. Accented characters are normalized (`á` → `a`), making searches case- and accent-insensitive.

**Query Parameters:**

| Param | Default | Description |
|---|---|---|
| `q` | — | Search query (one or more terms, space-separated) |
| `sort` | `desc` | Sort order: `asc` or `desc` |
| `take` | `20` | Max results to return (clamped 1–100) |
| `after` | — | Exclusive cursor ID for cursor-based pagination |

**Cursor-based pagination:** Omit `after` for the first page, then pass the last result's `id` as `after` for subsequent pages. In `desc` mode, `after` filters IDs lower than the cursor; in `asc` mode, it filters IDs higher.

**Response:** `200 OK`

```json
{
  "results": [
    { "id": "01HPT7B2X...", "content": "Rust and sled make a powerful, lightweight combination for embedded databases." }
  ],
  "total": 42,
  "take": 20
}

```

### 3. Suggest Words

`GET /collections/:collection/suggest?q=prefix`

Returns word-level autocomplete suggestions based on the last word in the query. Suggestions match indexed words by prefix — they do **not** consider sentence or phrase context. For example, searching `"application pro"` will suggest completions for `"pro"` (e.g. `"programming"`, `"process"`).

| Param | Default | Description |
|---|---|---|
| `q` | — | Word prefix to match (uses the last word if multiple) |

**Response:** `200 OK`

```json
{
  "suggestions": ["apple", "application", "apricot"]
}
```

### 4. Delete Item

`DELETE /collections/:collection/items/:id`

Removes the document from both the internal store and reverses its token mappings in the inverted index.

**Response:** `200 OK`

### 5. Delete Collection

`DELETE /collections/:collection`

Drops an entire collection index and its associated internal storage completely.

**Response:** `200 OK`

---

## Performance Optimization

* **SSD Native:** `sled` uses a log-structured architecture that avoids random write overhead, turning random operations into sequential disk writes, making it ideal for SSD flash memory.
* **Zero-Copy Serialization:** Internal storage uses efficient binary serialization to keep CPU overhead near zero during document ingestion and retrieval.
* **Fast Cleanups:** Because documents are stored internally by ID, deletion doesn't require sweeping the whole database; it targets only the exact words associated with that specific document.

## Local Development

Requires the [Rust toolchain](https://rustup.rs/).

```shell
cargo check     # Validate code compilation
cargo test      # Run the internal test suite
cargo run       # Start Aster in development mode

```

## License

This project is licensed under the MIT License - see the [LICENSE](https://www.google.com/search?q=LICENSE) file for details.
