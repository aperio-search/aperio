# Aperio — agent guide

## First-read warning

**The README is stale.** It says "sled" everywhere. The code uses **fjall** 3.x (LSM-tree). Treat README as conceptual reference only; the source in `src/` is the source of truth.

## Project structure

Single-crate Rust project (`aperio`). No workspace, no sub-crates.

```
src/
  main.rs       — entrypoint, reads DATA_DIR / CONFIG_FILE env vars, opens fjall DB, binds :3000
  lib.rs        — pub mod config, models, error, routes, store
  config.rs     — optional TOML config file parsing
  routes.rs     — Axum router with REST endpoints
  store.rs      — core engine: tokenization, inverted index, two ID strategies
tests/
  store.rs      — store integration tests (real fjall DB in tempdir)
  api.rs        — HTTP API integration tests via tower::ServiceExt
```

## Commands

```sh
cargo check              # compile-check only (fastest feedback)
cargo clippy             # lint (no custom config, uses defaults)
cargo fmt                # format (no custom config, uses rustfmt defaults)
cargo test               # runs all tests (unit + integration)
cargo test --lib         # unit tests only
cargo test --test api    # HTTP API integration tests only
cargo test --test store  # store integration tests only
cargo test <test_name>   # single test by name
cargo run                # dev server on :3000 (data persists to ./data/aperio)
cargo run --release      # optimized build
```

## Setup

- Rust toolchain: pinned via `rust-toolchain.toml` to channel `1.96`
- Edition 2024

## Runtime

Two environment variables control the server:

| Variable | Default | Description |
|---|---|---|
| `DATA_DIR` | `data` | Directory for persistent data (`{DATA_DIR}/aperio`) |
| `CONFIG_FILE` | (none) | Path to optional TOML config file |

Config file parsing is **strict**: on any read or parse error the process panics with a clear message. No fallback to defaults.

## Two ID strategies (store internals)

Collections are created with an `id_type`:
- **`string`** — posting lists stored as rkyv-archived shards (max_string_shard_size configurable)
- **`number`** — posting lists stored as RoaringTreemap bitmaps (max_roaring_shard_size configurable)

`POST /collections` with `{"name": "...", "id_type": "string" | "number"}`.

## Docs site

In `docs/` — VitePress, managed via npm. Lockfile is `docs/package-lock.lock`.

```sh
cd docs && npm install && npm run docs:dev
```

## Docker

Multi-stage build in `Dockerfile`. Image exposes `:3000`, expects `DATA_DIR=/data` and optional `CONFIG_FILE=/data/config.toml`. Verified via CI-less local build.

## License

Root: **Elastic License** (not MIT). `docs/` is MIT.

## Configuration changes

When adding, removing, or modifying any config option in `src/config.rs`, update the reference table and example in `docs/configuration.md` to match.

## What is NOT present

No CI workflows, no pre-commit hooks, no linter/formatter config files beyond defaults. No benchmarks. No generated code or codegen steps. No database migrations.

## Testing

Tests live in two places:

- **Unit tests** — `#[cfg(test)] mod tests` blocks at the bottom of `src/config.rs`, `src/error.rs`, `src/models.rs`, and `src/store/mod.rs`.
- **Integration tests** — standalone files in `tests/`.

All tests use the same dev-dependencies:

| Crate | Purpose |
|---|---|
| `tempfile` | Temporary directories for DB-backed store tests |
| `pretty_assertions` | Nicer assertion diffs |
| `tower` | `ServiceExt::oneshot` for HTTP test requests |
| `http-body-util` | Response body reading in error unit tests |

Key patterns:

- **Store unit tests** create a real `fjall::Database` in a `tempfile::TempDir` and construct a `Store` with it. The `TempDir` is kept alive alongside the `Store` so it isn't dropped early.
- **API tests** use `tower::ServiceExt::oneshot` on a cloned `axum::Router` (the router is cheap to clone; `oneshot` takes ownership).
- **Config tests** use `tempfile::NamedTempFile` to exercise the file-read path.
- **Error tests** use `tokio::runtime::Runtime::new().block_on()` to read response bodies synchronously.
