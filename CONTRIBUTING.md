# Contributing to Aperio

Thank you for your interest in contributing! This document covers the workflow and conventions for the project.

## Getting Started

- **Rust toolchain:** pinned to `1.96` via `rust-toolchain.toml` — it will be installed automatically by rustup.
- **No external services.** Aperio runs entirely locally with `fjall` for storage. Just clone and build.

```sh
cargo check
```

## Development Workflow

```sh
cargo check              # compile-check (fastest feedback)
cargo clippy             # lint (uses default config)
cargo fmt                # format (uses rustfmt defaults)
cargo test               # run all tests (unit + integration)
cargo test --lib         # unit tests only
cargo test --test api    # HTTP API integration tests only
cargo test --test store  # store integration tests only
cargo test <test_name>   # single test by name
cargo run                # dev server on :3000, data persists to ./data/aperio
```

### Runtime Configuration

| Variable | Default | Description |
|---|---|---|
| `DATA_DIR` | `data` | Directory for persistent data (`{DATA_DIR}/aperio`) |
| `CONFIG_FILE` | — | Path to optional TOML config file |

Config file parsing is **strict** — on any read or parse error the process panics with a clear message.

### Docs Site

Documentation lives in `docs/` (VitePress, managed via **npm**):

```sh
cd docs && npm install && npm run docs:dev
```

## Code Conventions

- **Edition 2024** Rust.
- Follow existing patterns in the codebase — this is a single-crate project with no workspaces.
- Keep dependencies minimal (check `Cargo.toml` before adding a new one).
- No custom linter/formatter config files are used — stick with `cargo clippy` and `cargo fmt` defaults.
- Do **not** add comments unless the logic genuinely requires explanation.
- When introducing new types or endpoints, match the naming and style of existing code in `src/`.

## Project Structure

```
src/
  main.rs       — entrypoint, reads env vars, opens fjall DB, binds :3000
  lib.rs        — pub mod declarations
  auth.rs       — API key authentication middleware
  backup.rs     — snapshot export/import
  config.rs     — optional TOML config parsing
  routes.rs     — Axum router with REST endpoints
  store/        — core engine: tokenization, inverted index, two ID strategies (mod.rs, config.rs, posting_list.rs, search.rs)
tests/
  store.rs      — store integration tests (real fjall DB in tempdir)
  api.rs        — HTTP API integration tests via tower::ServiceExt
```

See `AGENTS.md` for more detailed internals documentation.

## Licensing

- **Source code** (`src/`, `Cargo.toml`, `Dockerfile`, etc.) is licensed under the **Elastic License** — see [`LICENSE`](LICENSE).
- **Documentation** (`docs/`) is MIT — see [`docs/LICENSE`](docs/LICENSE).

By contributing, you agree that your contributions will be licensed under the Elastic License (source) or MIT (docs) as applicable.
