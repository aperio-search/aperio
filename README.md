<h1 align="center">Aperio</h1>

<p align="center">
<a href="https://github.com/aperio-search/aperio"><img src="https://img.shields.io/badge/aperio-Screamingly%20fast-red" alt="Aperio" height=50></a>
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

### Build from source

```bash
git clone https://github.com/aperio-search/aperio.git
cd aperio
cargo build --release
./target/release/aperio
```

## Quickstart

```bash
cargo run --release
# server starts on http://0.0.0.0:3000

# create a collection
curl -X POST http://localhost:3000/collections \
  -H 'Content-Type: application/json' \
  -d '{"name":"posts","id_type":"string"}'

# index a document
curl -X POST http://localhost:3000/collections/posts/items \
  -H 'Content-Type: application/json' \
  -d '{"id":"1","content":"Hello world from Aperio"}'

# search
curl "http://localhost:3000/collections/posts/search?q=hello&take=10"
```

## Quick links

- Search
  - [Inserting items](https://aperiosearch.com/inserting-items.html)
  - [Searching](https://aperiosearch.com/search.html)
  - [Autocomplete / suggest](https://aperiosearch.com/autocomplete.html)
  - [Delete items](https://aperiosearch.com/deleting-items.html)

- Collections
  - [Create a collection](https://aperiosearch.com/creating-collections.html)
  - [Collection metadata](https://aperiosearch.com/collection-metadata.html)
  - [List collections](https://aperiosearch.com/listing-collections.html)
  - [Delete a collection](https://aperiosearch.com/listing-collections.html)

- Configuration
  - [Environment variables (`DATA_DIR`, `CONFIG_FILE`)](https://aperiosearch.com/configuration.html)

## Contributing

See [CONTRIBUTING](https://github.com/aperio-search/aperio/blob/main/CONTRIBUTING.md) to get started.
