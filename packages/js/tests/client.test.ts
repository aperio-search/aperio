import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, it, mock } from "node:test";
import { AperioClient, AperioError, toCamel } from "../src/index.js";

describe("AperioClient", () => {
	let client: AperioClient;

	beforeEach(() => {
		client = new AperioClient({
			baseUrl: "http://localhost:3000",
			apiKey: "test-key",
		});
	});

	afterEach(() => {
		mock.restoreAll();
	});

	describe("status", () => {
		it("returns ok from server", async () => {
			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(JSON.stringify({ ok: true })),
					json: () => Promise.resolve({ ok: true }),
				}),
			);

			const result = await client.status();
			assert.strictEqual(result.ok, true);
		});
	});

	describe("listCollections", () => {
		it("returns collections", async () => {
			const body = {
				collections: [
					{
						name: "posts",
						id_type: "number",
						searchable_fields: ["title"],
					},
				],
			};

			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(JSON.stringify(body)),
					json: () => Promise.resolve(body),
				}),
			);

			const result = await client.listCollections();
			assert.strictEqual(result.collections.length, 1);
			assert.strictEqual(result.collections[0].name, "posts");
			assert.strictEqual(result.collections[0].idType, "number");
			assert.deepStrictEqual(result.collections[0].searchableFields, ["title"]);
		});
	});

	describe("createCollection", () => {
		it("sends camelCase and receives camelCase", async () => {
			const body = {
				name: "messages",
				id_type: "number",
				searchable_fields: ["title", "body"],
			};

			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 201,
					text: () => Promise.resolve(JSON.stringify(body)),
					json: () => Promise.resolve(body),
				}),
			);

			const result = await client.createCollection({
				name: "messages",
				idType: "number",
				searchableFields: ["title", "body"],
			});

			assert.strictEqual(result.name, "messages");
			assert.strictEqual(result.idType, "number");
			assert.deepStrictEqual(result.searchableFields, ["title", "body"]);

			const {
				arguments: [url, opts],
			} = (globalThis.fetch as ReturnType<typeof mock.fn>).mock.calls[0];
			assert.strictEqual(opts.method, "POST");
			assert.ok(url.includes("/collections"));
			const sent = JSON.parse(opts.body);
			assert.strictEqual(sent.id_type, "number");
			assert.deepStrictEqual(sent.searchable_fields, ["title", "body"]);
		});
	});

	describe("getCollection", () => {
		it("returns collection info", async () => {
			const body = {
				name: "posts",
				id_type: "number",
				document_count: 42,
				searchable_fields: ["title"],
			};

			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(JSON.stringify(body)),
					json: () => Promise.resolve(body),
				}),
			);

			const result = await client.getCollection("posts");
			assert.strictEqual(result.name, "posts");
			assert.strictEqual(result.documentCount, 42);
		});
	});

	describe("deleteCollection", () => {
		it("sends DELETE request", async () => {
			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(""),
					json: () => Promise.resolve({}),
				}),
			);

			await client.deleteCollection("posts");

			const {
				arguments: [url, opts],
			} = (globalThis.fetch as ReturnType<typeof mock.fn>).mock.calls[0];
			assert.strictEqual(opts.method, "DELETE");
			assert.ok(url.includes("/collections/posts"));
		});
	});

	describe("upsertItem", () => {
		it("sends POST with document", async () => {
			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(""),
					json: () => Promise.resolve({}),
				}),
			);

			const doc = { id: "abc", title: "Hello" };
			await client.upsertItem("posts", doc);

			const {
				arguments: [url, opts],
			} = (globalThis.fetch as ReturnType<typeof mock.fn>).mock.calls[0];
			assert.strictEqual(opts.method, "POST");
			assert.ok(url.includes("/collections/posts/items"));
			assert.deepStrictEqual(JSON.parse(opts.body), doc);
		});
	});

	describe("deleteItem", () => {
		it("sends DELETE with id", async () => {
			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(""),
					json: () => Promise.resolve({}),
				}),
			);

			await client.deleteItem("posts", "abc123");

			const {
				arguments: [url, opts],
			} = (globalThis.fetch as ReturnType<typeof mock.fn>).mock.calls[0];
			assert.strictEqual(opts.method, "DELETE");
			assert.ok(url.includes("/collections/posts/items/abc123"));
		});
	});

	describe("search", () => {
		it("sends query params and returns camelCase", async () => {
			const body = {
				results: [{ id: "1", title: "hello" }],
				take: 20,
				elapsed_ms: 1.234,
			};

			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(JSON.stringify(body)),
					json: () => Promise.resolve(body),
				}),
			);

			const result = await client.search("posts", {
				q: "hello",
				sort: "asc",
				take: 50,
				after: "abc",
			});

			assert.deepStrictEqual(result.results, [{ id: "1", title: "hello" }]);
			assert.strictEqual(result.take, 20);
			assert.strictEqual(result.elapsedMs, 1.234);

			const {
				arguments: [url],
			} = (globalThis.fetch as ReturnType<typeof mock.fn>).mock.calls[0];
			assert.ok(url.includes("q=hello"));
			assert.ok(url.includes("sort=asc"));
			assert.ok(url.includes("take=50"));
			assert.ok(url.includes("after=abc"));
		});

		it("defaults to desc sort when not provided", async () => {
			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () =>
						Promise.resolve(
							JSON.stringify({ results: [], take: 20, elapsed_ms: 0 }),
						),
					json: () => Promise.resolve({ results: [], take: 20, elapsed_ms: 0 }),
				}),
			);

			await client.search("posts", { q: "test" });

			const {
				arguments: [url],
			} = (globalThis.fetch as ReturnType<typeof mock.fn>).mock.calls[0];
			assert.ok(url.includes("q=test"));
			assert.ok(!url.includes("sort"));
		});
	});

	describe("queueDepth", () => {
		it("returns pending count", async () => {
			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: true,
					status: 200,
					text: () => Promise.resolve(JSON.stringify({ pending: 5 })),
					json: () => Promise.resolve({ pending: 5 }),
				}),
			);

			const result = await client.queueDepth();
			assert.strictEqual(result.pending, 5);
		});
	});

	describe("error handling", () => {
		it("throws AperioError on 4xx", async () => {
			globalThis.fetch = mock.fn(() =>
				Promise.resolve({
					ok: false,
					status: 404,
					text: () => Promise.resolve(JSON.stringify({ error: "not found" })),
					json: () => Promise.resolve({ error: "not found" }),
				}),
			);

			await assert.rejects(
				() => client.getCollection("nope"),
				(err: unknown) => {
					assert.ok(err instanceof AperioError);
					assert.strictEqual((err as AperioError).status, 404);
					assert.strictEqual((err as AperioError).message, "not found");
					return true;
				},
			);
		});
	});

	describe("toCamel", () => {
		it("converts snake_case keys to camelCase", () => {
			const result = toCamel({
				id_type: "number",
				searchable_fields: ["title"],
				document_count: 42,
			});
			assert.deepStrictEqual(result, {
				idType: "number",
				searchableFields: ["title"],
				documentCount: 42,
			});
		});

		it("handles nested objects", () => {
			const result = toCamel({
				collection_info: {
					document_count: 5,
				},
			});
			assert.deepStrictEqual(result, {
				collectionInfo: {
					documentCount: 5,
				},
			});
		});

		it("handles arrays of objects", () => {
			const result = toCamel({
				collections: [{ id_type: "string", searchable_fields: ["body"] }],
			});
			assert.deepStrictEqual(result, {
				collections: [{ idType: "string", searchableFields: ["body"] }],
			});
		});
	});
});
