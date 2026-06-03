# @aperio-search/aperio

Node.js client for [Aperio](https://aperiosearch.com) — a screamingly fast, ultra-lightweight search engine.

## Installation

```sh
npm install @aperio-search/aperio
```

## Usage

```ts
import { AperioClient } from "@aperio-search/aperio";

const client = new AperioClient({
  baseUrl: "http://localhost:3000",
  apiKey: "SecretApiKey",
});

// Create a collection
await client.createCollection({
  name: "movies",
  idType: "string",
  searchableFields: ["title"],
});

// Index documents
await client.upsertItem("movies", { id: "1", title: "the empire strikes back" });
await client.upsertItem("movies", { id: "2", title: "star wars a new hope" });
await client.upsertItem("movies", { id: "3", title: "return of the jedi" });

// Search
const { results } = await client.search("movies", { q: "star" });

// Delete
await client.deleteItem("movies", "1");
await client.deleteCollection("movies");
```

## API

### Collections

| Method | Description |
|--------|-------------|
| `listCollections()` | List all collections |
| `createCollection(req)` | Create a new collection |
| `getCollection(name)` | Get collection metadata |
| `deleteCollection(name)` | Delete a collection |

### Documents

| Method | Description |
|--------|-------------|
| `upsertItem(collection, doc)` | Insert or update a document |
| `deleteItem(collection, id)` | Delete a document by ID |

### Search

| Method | Description |
|--------|-------------|
| `search(collection, params)` | Full-text search with pagination |
| `suggest(collection, params)` | Suggest indexed terms matching a prefix |

### Backup

| Method | Description |
|--------|-------------|
| `exportBackup()` | Export a database snapshot |
| `importBackup(name)` | Import a database snapshot |

### Health

| Method | Description |
|--------|-------------|
| `status()` | Server health check |

## License

MIT
