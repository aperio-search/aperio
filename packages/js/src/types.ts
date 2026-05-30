export interface ClientOptions {
	baseUrl: string;
	apiKey: string;
}

export interface CollectionSummary {
	name: string;
	idType: string;
	searchableFields: string[];
}

export interface ListCollectionsResponse {
	collections: CollectionSummary[];
}

export interface CreateCollectionRequest {
	name: string;
	idType: string;
	searchableFields: string[];
}

export interface CollectionCreated {
	name: string;
	idType: string;
	searchableFields: string[];
}

export interface CollectionInfo {
	name: string;
	idType: string;
	documentCount: number;
	uniqueTerms: number;
	searchableFields: string[];
}

export interface SearchParams {
	q: string;
	sort?: "asc" | "desc";
	take?: number;
	after?: string;
}

export interface SearchResponse {
	results: Record<string, unknown>[];
	take: number;
	elapsedMs: number;
}

export interface SuggestParams {
	q: string;
}

export interface SuggestResponse {
	suggestions: string[];
}

export interface ExportResponse {
	ok: boolean;
	size: number;
	file: string;
}

export interface ImportResponse {
	ok: boolean;
}

export interface StatusResponse {
	ok: boolean;
}

export class AperioError extends Error {
	status: number;

	constructor(status: number, message: string) {
		super(message);
		this.name = "AperioError";
		this.status = status;
	}
}

type JsonValue =
	| string
	| number
	| boolean
	| null
	| JsonValue[]
	| { [key: string]: JsonValue };

export function toCamel(
	obj: Record<string, JsonValue>,
): Record<string, JsonValue> {
	const result: Record<string, JsonValue> = {};
	for (const [key, value] of Object.entries(obj)) {
		const camelKey = key.replace(/_([a-z])/g, (_, c) => c.toUpperCase());
		if (
			Array.isArray(value) &&
			value.every((v) => typeof v === "object" && v !== null)
		) {
			result[camelKey] = value.map((v) =>
				toCamel(v as Record<string, JsonValue>),
			);
		} else if (
			typeof value === "object" &&
			value !== null &&
			!Array.isArray(value)
		) {
			result[camelKey] = toCamel(value as Record<string, JsonValue>);
		} else {
			result[camelKey] = value;
		}
	}
	return result;
}
