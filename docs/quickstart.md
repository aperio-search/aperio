# Quickstart

> If you have a few seconds, [a star on GitHub helps us a lot](https://github.com/andresribeiro/aster)!

Aster is an screamingly fast search engine designed to use minimal resource consumption. To maintain this extreme efficiency and high throughput, Aster bypasses heavy, resource-intensive features in favor of a lean architecture:

- **Ordering**: only supported by `ID ASC` or `ID DESC`;
- **Filtering**: Not supported directly. However, you can easily bypass this by structuring your collections. For example, if you want to search messages by a specific user, you can simply create a dedicated collection named `messages:[userId]`.
- **Search Relevance**: Not supported. If you search for a term, Aster will return all documents containing that term, but it won't rank them.

## Installation

```sh
docker run -e DATA_DIR=/data -p 3000:3000 --name aster andresribeiro/aster
```

| Param | Description
|---|---
| `-e DATA_DIR=/data` | Defines the internal directory where Aster will persist its search index and data files.
| `-p 3000:3000` | Exposes the Aster API, mapping port 3000 of the container to port 3000.
| `--name aster` | Assigns a memorable, custom name to the container for easier management.

Aster will be reachable on `http://localhost:3000`.
