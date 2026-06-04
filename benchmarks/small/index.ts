import fs from "node:fs";
import readline from "node:readline";

const COLLECTION_NAME: string = "books";
const DATA_FILE_PATH: string = "./books.csv";
const BATCH_SIZE: number = 500;
const API_SECRET: string = "SecretApiKey";

const BASE_URL = "http://localhost:3000";
const HEADERS: Record<string, string> = {
  "Content-Type": "application/json",
  Authorization: API_SECRET,
};

interface HttpError extends Error {
  status?: number;
}

interface BookPayload {
  id: number;
  title: string;
  authors: string;
  average_rating: number | null;
  isbn: string;
  isbn13: string;
  language_code: string;
  num_pages: number | null;
  ratings_count: number | null;
  text_reviews_count: number | null;
  publication_date: string;
  publisher: string;
}

async function apiPost(path: string, body: object): Promise<void> {
  const res = await fetch(`${BASE_URL}${path}`, {
    method: "POST",
    headers: HEADERS,
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const err: HttpError = new Error(`HTTP ${res.status}`);
    err.status = res.status;
    throw err;
  }
}

async function createCollection(): Promise<void> {
  try {
    console.log(`Setting up collection: "${COLLECTION_NAME}"...`);
    await apiPost("/collections", {
      name: COLLECTION_NAME,
      id_type: "number",
      searchable_fields: ["title", "authors", "publisher"],
    });
    console.log("Collection verified/ready.");
  } catch (error) {
    const err = error as HttpError;
    if (err.status === 409) {
      console.log("Collection already exists. Moving to indexing...");
    } else if (err.status === 401) {
      console.error('Authentication failed! Check your "SecretApiKey".');
      process.exit(1);
    } else {
      console.error("Failed to create collection:", err.message);
      process.exit(1);
    }
  }
}

function parseCSVLine(line: string): string[] {
  const result: string[] = [];
  let current: string = "";
  let inQuotes: boolean = false;

  for (let i = 0; i < line.length; i++) {
    const char = line[i];
    if (char === '"') {
      inQuotes = !inQuotes;
    } else if (char === "," && !inQuotes) {
      result.push(current);
      current = "";
    } else {
      current += char;
    }
  }
  result.push(current);
  return result;
}

async function sendBatch(batch: BookPayload[]): Promise<number> {
  const res = await fetch(`${BASE_URL}/collections/${COLLECTION_NAME}/items/bulk`, {
    method: "POST",
    headers: HEADERS,
    body: JSON.stringify(batch),
  });
  if (!res.ok) {
    const err: HttpError = new Error(`HTTP ${res.status}`);
    err.status = res.status;
    throw err;
  }
  const body = await res.json() as { ok: boolean; count: number };
  return body.count;
}

async function indexBooks(): Promise<void> {
  const fileStream = fs.createReadStream(DATA_FILE_PATH);
  const rl = readline.createInterface({
    input: fileStream,
    crlfDelay: Infinity,
  });

  let totalLinesRead: number = 0;
  let successCount: number = 0;
  let failureCount: number = 0;
  let isHeader: boolean = true;
  let batch: BookPayload[] = [];

  console.log(`Starting bulk ingestion (Batches of ${BATCH_SIZE})...`);
  const startTime: number = Date.now();

  for await (const line of rl) {
    if (isHeader) {
      isHeader = false;
      continue;
    }

    const columns: string[] = parseCSVLine(line);
    if (columns.length < 12) continue;

    totalLinesRead++;

    const [
      bookID,
      title,
      authors,
      average_rating,
      isbn,
      isbn13,
      language_code,
      num_pages,
      ratings_count,
      text_reviews_count,
      publication_date,
      publisher,
    ] = columns;

    const payload: BookPayload = {
      id: totalLinesRead,
      title: title || "",
      authors: authors || "",
      average_rating: average_rating ? parseFloat(average_rating) : null,
      isbn: isbn || "",
      isbn13: isbn13 || "",
      language_code: language_code || "",
      num_pages: num_pages ? parseInt(num_pages, 10) : null,
      ratings_count: ratings_count ? parseInt(ratings_count, 10) : null,
      text_reviews_count: text_reviews_count
        ? parseInt(text_reviews_count, 10)
        : null,
      publication_date: publication_date || "",
      publisher: publisher || "",
    };

    batch.push(payload);

    if (batch.length === BATCH_SIZE) {
      try {
        const count = await sendBatch(batch);
        successCount += count;
      } catch (err: any) {
        failureCount += batch.length;
        if (failureCount === BATCH_SIZE && err.status === 401) {
          console.error(
            "The server rejected a bulk write with a 401 Unauthorized status.",
          );
        }
      }
      batch = [];

      if (successCount % 1000 === 0) {
        const elapsedMin: string = (
          (Date.now() - startTime) /
          1000 /
          60
        ).toFixed(2);
        console.log(
          `Speedometer: ${successCount} indexed. (Read: ${totalLinesRead} lines, Elapsed: ${elapsedMin} min)`,
        );
      }
    }
  }

  if (batch.length > 0) {
    try {
      const count = await sendBatch(batch);
      successCount += count;
    } catch {
      failureCount += batch.length;
    }
  }

  const totalTimeMin: string = ((Date.now() - startTime) / 1000 / 60).toFixed(
    2,
  );
  console.log(`\n--- Batch Indexing Complete ---`);
  console.log(`Execution Time:     ${totalTimeMin} minutes`);
  console.log(`Total Rows Parsed:  ${totalLinesRead}`);
  console.log(`Engine Accepted:    ${successCount}`);
  console.log(`Engine Rejected:    ${failureCount}`);
}

async function main(): Promise<void> {
  await createCollection();
  await indexBooks();
}

main();
