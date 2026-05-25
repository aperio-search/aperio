# Aster — AGENTS.md

## Project structure

Single Rust binary (`edition 2024`), no workspace, no tests yet. Server starts on port **3000**, data dir defaults to `./data` (overridable via `DATA_DIR` env var). Database file is `{DATA_DIR}/aster.db`.

- `src/main.rs` — entrypoint; opens sled DB, creates `Store`, binds `0.0.0.0:3000`
- `src/store.rs` — core indexing logic; uses CAS (`compare_and_swap`) for lock-free concurrent upsert/delete
- `src/routes.rs` — axum router with 5 endpoints + fallback
- `src/error.rs` — custom `AppError` → JSON error responses
- `src/models.rs` — request/response structs

## Key deps (Cargo.toml)

| Crate | Purpose |
|---|---|
| `axum 0.8` | HTTP framework |
| `sled 0.34` | Embedded KV store (disk-backed) |
| `unicode-normalization 0.1` | NFKD + combining-mark stripping |
| `serde / serde_json` | JSON wire format for API & posting lists |

## Commands

```sh
cargo run          # dev server on :3000, ephemeral ./data/aster.db
cargo run --release # optimized build
cargo check        # fast compile check
```

There are **no tests** (`cargo test` produces nothing). No linter/formatter config exists.

## API endpoints

| Method | Path | Body/Query |
|---|---|---|
| `POST`   | `/collections/{collection}/items` | `{ "id": "...", "content": "..." }` |
| `GET`    | `/collections/{collection}/search` | `?q=term&sort=desc&take=20&after=` — returns `{ results: [id, ...], total, take }` |
| `GET`    | `/collections/{collection}/suggest` | `?q=prefix` |
| `DELETE` | `/collections/{collection}/items/{id}` | — |
| `DELETE` | `/collections/{collection}` | — |
| `GET`    | `/status` | — |

## Query quirks

- Search is **AND-only** (multiple terms, all must match). No OR, no filtering.
- Sort is by **document ID lexicographic order**, default `DESC`. Use ULID/UUIDv7/zero-padded IDs for predictable ordering.
- Cursor pagination: `after` is exclusive — in `desc` mode filters IDs < cursor, in `asc` mode filters IDs > cursor. `take` clamped 1–100.
- Suggest returns up to **10 prefix matches** from the inverted index (uses `sled::Tree::scan_prefix`).
- Tokenization: NFKD normalize → strip combining marks → lowercase → strip non-alphanumeric (configurable via `StoreConfig::strip_punctuation`) → filter tokens shorter than `min_token_length` (default 2).

## Storage layout

Each collection uses two sled trees: `{collection}:inverted` (word→[doc IDs]) and `{collection}:docs` (doc ID→[tokens] — JSON array of normalized tokens). See `store.rs:39-45`.

## Docker

```sh
docker build -t aster .
docker run -e DATA_DIR=/data -p 3000:3000 aster
```

`Dockerfile` uses `rust:1.85-slim-bookworm` to build, `debian:bookworm-slim` at runtime. Binary lives at `/aster`.

## Style notes

- No comments in code — match that convention when editing.
- `AppError` converts `sled::Error` to `Internal` automatically (`error.rs:22`).
- All endpoints return JSON errors with shape `{ "error": "..." }`.
