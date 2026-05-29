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
```

## Commands

```sh
cargo check              # compile-check only (fastest feedback)
cargo clippy             # lint (no custom config, uses defaults)
cargo fmt                # format (no custom config, uses rustfmt defaults)
cargo test               # runs — but there are zero tests in the codebase
cargo run                # dev server on :3000 (data persists to ./data/aperio.db)
cargo run --release      # optimized build
```

## Setup

- Rust toolchain: pinned via `rust-toolchain.toml` to channel `1.95`
- Edition 2024

## Runtime

Two environment variables control the server:

| Variable | Default | Description |
|---|---|---|
| `DATA_DIR` | `data` | Directory for persistent data (`{DATA_DIR}/aperio.db`) |
| `CONFIG_FILE` | (none) | Path to optional TOML config file |

Config file parsing is **silently lenient**: on any read/parse error it falls back to defaults with only a warning to stderr. No hard failures.

## Two ID strategies (store internals)

Collections are created with an `id_type`:
- **`string`** — posting lists stored as rkyv-archived shards (max_shard_size configurable)
- **`number`** — posting lists stored as RoaringTreemap bitmaps (max_roaring_shard_size configurable)

`POST /collections` with `{"name": "...", "id_type": "string" | "number"}`.

## Docs site

In `docs/` — VitePress, managed via **bun** (not npm). Lockfile is `docs/bun.lock`.

```sh
cd docs && bun install && bun run docs:dev
```

## Docker

Multi-stage build in `Dockerfile`. Image exposes `:3000`, expects `DATA_DIR=/data` and optional `CONFIG_FILE=/data/config.toml`. Verified via CI-less local build.

## License

Root: **Elastic License** (not MIT). `docs/` is MIT.

## What is NOT present

No CI workflows, no pre-commit hooks, no linter/formatter config files beyond defaults. No integration tests, no benchmarks. No generated code or codegen steps. No database migrations.
