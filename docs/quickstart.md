# Quickstart

> If you have a few seconds, [a star on GitHub helps us a lot](https://github.com/andresribeiro/aperio)!

Aperio is an screamingly fast search engine designed to use minimal resource consumption. To maintain this extreme efficiency and high throughput, Aperio bypasses heavy, resource-intensive features in favor of a lean architecture:

- **Ordering**: only supported by `ID ASC` or `ID DESC`;
- **Filtering**: Not supported directly. However, you can easily bypass this by structuring your collections. For example, if you want to search messages by a specific user, you can simply create a dedicated collection named `messages:[userId]`.
- **Search Relevance**: Not supported. If you search for a term, Aperio will return all documents containing that term, but it won't rank them.

## Installation

```sh
docker run -e DATA_DIR=/data -p 3000:3000 --name aperio andresribeiro/aperio
```

| Param | Description
|---|---
| `-e DATA_DIR=/data` | Defines the internal directory where Aperio will persist its search index and data files.
| `-p 3000:3000` | Exposes the Aperio API, mapping port 3000 of the container to port 3000.
| `--name aperio` | Assigns a memorable, custom name to the container for easier management.

Aperio will be reachable on `http://localhost:3000`.
