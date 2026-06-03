import {
	AperioError,
	type ClientOptions,
	type CollectionCreated,
	type CollectionInfo,
	type CreateCollectionRequest,
	type ExportResponse,
	type ImportResponse,
	type ListCollectionsResponse,
	type QueueDepthResponse,
	type SearchParams,
	type SearchResponse,
	type StatusResponse,
	type SuggestParams,
	type SuggestResponse,
	toCamel,
} from "./types.js";

type JsonValue =
	| string
	| number
	| boolean
	| null
	| JsonValue[]
	| { [key: string]: JsonValue };

function toSnake(obj: Record<string, JsonValue>): Record<string, JsonValue> {
	const result: Record<string, JsonValue> = {};
	for (const [key, value] of Object.entries(obj)) {
		const snakeKey = key.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);
		if (Array.isArray(value)) {
			result[snakeKey] = value;
		} else if (typeof value === "object" && value !== null) {
			result[snakeKey] = toSnake(value as Record<string, JsonValue>);
		} else {
			result[snakeKey] = value;
		}
	}
	return result;
}

export class AperioClient {
	#baseUrl: string;
	#apiKey: string;

	constructor(options: ClientOptions) {
		this.#baseUrl = options.baseUrl.replace(/\/+$/, "");
		this.#apiKey = options.apiKey;
	}

	async #request(
		method: string,
		path: string,
		body?: Record<string, JsonValue>,
	): Promise<Record<string, JsonValue>> {
		const url = new URL(path, this.#baseUrl);
		const headers: Record<string, string> = {
			Authorization: this.#apiKey,
		};

		let bodyStr: string | undefined;
		if (body !== undefined) {
			headers["Content-Type"] = "application/json";
			bodyStr = JSON.stringify(toSnake(body));
		}

		const response = await fetch(url.toString(), {
			method,
			headers,
			body: bodyStr,
		});

		if (!response.ok) {
			let message = `HTTP ${response.status}`;
			try {
				const errBody = (await response.json()) as Record<string, JsonValue>;
				if (typeof errBody.error === "string") {
					message = errBody.error;
				}
			} catch {
				// ignore parse errors
			}
			throw new AperioError(response.status, message);
		}

		const text = await response.text();
		if (text.length === 0) {
			return {};
		}
		return JSON.parse(text) as Record<string, JsonValue>;
	}

	async status(): Promise<StatusResponse> {
		const data = await this.#request("GET", "/status");
		return { ok: Boolean(data.ok) };
	}

	async listCollections(): Promise<ListCollectionsResponse> {
		const data = await this.#request("GET", "/collections");
		const raw = toCamel(data);
		return raw as unknown as ListCollectionsResponse;
	}

	async createCollection(
		req: CreateCollectionRequest,
	): Promise<CollectionCreated> {
		const data = await this.#request(
			"POST",
			"/collections",
			req as unknown as Record<string, JsonValue>,
		);
		return toCamel(data) as unknown as CollectionCreated;
	}

	async getCollection(name: string): Promise<CollectionInfo> {
		const data = await this.#request(
			"GET",
			`/collections/${encodeURIComponent(name)}`,
		);
		return toCamel(data) as unknown as CollectionInfo;
	}

	async deleteCollection(name: string): Promise<void> {
		await this.#request("DELETE", `/collections/${encodeURIComponent(name)}`);
	}

	async upsertItem(
		collection: string,
		doc: Record<string, unknown>,
	): Promise<void> {
		await this.#request(
			"POST",
			`/collections/${encodeURIComponent(collection)}/items`,
			doc as Record<string, JsonValue>,
		);
	}

	async deleteItem(collection: string, id: string): Promise<void> {
		await this.#request(
			"DELETE",
			`/collections/${encodeURIComponent(collection)}/items/${encodeURIComponent(id)}`,
		);
	}

	async search(
		collection: string,
		params: SearchParams,
	): Promise<SearchResponse> {
		const query = new URLSearchParams();
		query.set("q", params.q);
		if (params.sort !== undefined) query.set("sort", params.sort);
		if (params.take !== undefined) query.set("take", String(params.take));
		if (params.after !== undefined) query.set("after", params.after);

		const url = `/collections/${encodeURIComponent(collection)}/search?${query.toString()}`;
		const data = await this.#request("GET", url);
		return toCamel(data) as unknown as SearchResponse;
	}

	async suggest(
		collection: string,
		params: SuggestParams,
	): Promise<SuggestResponse> {
		const query = new URLSearchParams();
		query.set("q", params.q);
		if (params.take !== undefined) query.set("take", String(params.take));

		const url = `/collections/${encodeURIComponent(collection)}/suggest?${query.toString()}`;
		const data = await this.#request("GET", url);
		return toCamel(data) as unknown as SuggestResponse;
	}

	async queueDepth(): Promise<QueueDepthResponse> {
		const data = await this.#request("GET", "/queue");
		return toCamel(data) as unknown as QueueDepthResponse;
	}

	async exportBackup(): Promise<ExportResponse> {
		const data = await this.#request("POST", "/backup/export");
		return toCamel(data) as unknown as ExportResponse;
	}

	async importBackup(name: string): Promise<ImportResponse> {
		const data = await this.#request("POST", "/backup/import", { name });
		return toCamel(data) as unknown as ImportResponse;
	}
}
