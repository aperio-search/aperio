# Quickstart

> If you have a few seconds, [a star on GitHub helps us a lot](https://github.com/aperio-search/aperio)!

Aperio is an screamingly fast search engine designed to use minimal resource consumption. To maintain this extreme efficiency and high throughput, Aperio bypasses heavy, resource-intensive features in favor of a lean architecture:

- **Ordering**: only supported by `ID ASC` or `ID DESC`;
- **Filtering**: Not supported directly. However, you can easily bypass this by structuring your collections. For example, if you want to search messages by a specific user, you can simply create a dedicated collection named `messages:[userId]`.
- **Search Relevance**: Not supported. If you search for a term, Aperio will return all documents containing that term, but it won't rank them.

## Installation

```sh
docker run -e DATA_DIR=/data -p 3000:3000 --name aperio andresribeiro/aperio
```

| Param | Description
|---|---|
| `-e DATA_DIR=/data` | Defines the internal directory where Aperio will persist its search index and data files.
| `-p 3000:3000` | Exposes the Aperio API, mapping port 3000 of the container to port 3000.
| `--name aperio` | Assigns a memorable, custom name to the container for easier management.

Aperio will be reachable on `http://localhost:3000`.

## Try it out

Once the server is running, create a collection and start searching.

::: code-group

```js [Node.js]
// npm install @aperio-search/aperio

import { AperioClient } from "@aperio-search/aperio";

const client = new AperioClient({
  baseUrl: "http://localhost:3000",
  apiKey: "SecretApiKey",
});

await client.createCollection({
  name: "movies",
  idType: "string",
  searchableFields: ["title"],
});

await client.upsertItem("movies", { id: "1", title: "the empire strikes back" });
await client.upsertItem("movies", { id: "2", title: "star wars a new hope" });
await client.upsertItem("movies", { id: "3", title: "return of the jedi" });

const results = await client.search("movies", { q: "star" });
```

```js [Fetch]
// Create the "movies" collection
await fetch("http://localhost:3000/collections", {
  method: "POST",
  headers: { "Content-Type": "application/json", Authorization: "SecretApiKey" },
  body: JSON.stringify({ name: "movies", id_type: "string", searchable_fields: ["title"] }),
});

// Insert a few documents
await fetch("http://localhost:3000/collections/movies/items", {
  method: "POST",
  headers: { "Content-Type": "application/json", Authorization: "SecretApiKey" },
  body: JSON.stringify({ id: "1", title: "the empire strikes back" }),
});
await fetch("http://localhost:3000/collections/movies/items", {
  method: "POST",
  headers: { "Content-Type": "application/json", Authorization: "SecretApiKey" },
  body: JSON.stringify({ id: "2", title: "star wars a new hope" }),
});
await fetch("http://localhost:3000/collections/movies/items", {
  method: "POST",
  headers: { "Content-Type": "application/json", Authorization: "SecretApiKey" },
  body: JSON.stringify({ id: "3", title: "return of the jedi" }),
});

// Search for "star"
const res = await fetch("http://localhost:3000/collections/movies/search?q=star", {
  headers: { Authorization: "PublicApiKey" },
});
const movies = await res.json();
```

```sh [cURL]
# Create a collection that uses string IDs, with "title" as the searchable field
curl -X POST http://localhost:3000/collections \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"name": "movies", "id_type": "string", "searchable_fields": ["title"]}'

# Insert a few documents
curl -X POST http://localhost:3000/collections/movies/items \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"id": "1", "title": "the empire strikes back"}'

curl -X POST http://localhost:3000/collections/movies/items \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"id": "2", "title": "star wars a new hope"}'

curl -X POST http://localhost:3000/collections/movies/items \
  -H "Content-Type: application/json" \
  -H "Authorization: SecretApiKey" \
  -d '{"id": "3", "title": "return of the jedi"}'

# Search for "star" — returns the full matching document(s)
curl "http://localhost:3000/collections/movies/search?q=star" \
  -H "Authorization: PublicApiKey"
```
