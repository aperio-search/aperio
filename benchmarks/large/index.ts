import fs from "node:fs";
import readline from "node:readline";

const COLLECTION_NAME = "imdb_titles";
const DATA_FILE_PATH = "./title.basics.tsv";
const BATCH_SIZE = 500;
const API_SECRET = "SecretApiKey";

const BASE_URL = "http://localhost:3000";
const HEADERS = {
  "Content-Type": "application/json",
  Authorization: API_SECRET,
};

async function apiPost(path: string, body: object): Promise<number> {
  const res = await fetch(`${BASE_URL}${path}`, {
    method: "POST",
    headers: HEADERS,
    body: JSON.stringify(body),
  });
  return res.status;
}

async function createCollection() {
  try {
    console.log(`Setting up collection: "${COLLECTION_NAME}"...`);
    const status = await apiPost("/collections", {
      name: COLLECTION_NAME,
      id_type: "number",
      searchable_fields: ["title", "type", "genres"],
    });
    if (status === 409) {
      console.log("Collection already exists. Moving to indexing...");
    } else if (status === 401) {
      console.error('Authentication failed! Check your "SecretApiKey".');
      process.exit(1);
    } else if (status < 200 || status >= 300) {
      throw new Error(`HTTP ${status}`);
    } else {
      console.log("Collection verified/ready.");
    }
  } catch (error) {
    console.error("Failed to create collection:", (error as Error).message);
    process.exit(1);
  }
}

async function sendBatch(batch: object[]): Promise<number> {
  const res = await fetch(`${BASE_URL}/collections/${COLLECTION_NAME}/items/bulk`, {
    method: "POST",
    headers: HEADERS,
    body: JSON.stringify(batch),
  });
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }
  const body = await res.json() as { ok: boolean; count: number };
  return body.count;
}

async function indexImdbTitles() {
  const fileStream = fs.createReadStream(DATA_FILE_PATH);
  const rl = readline.createInterface({
    input: fileStream,
    crlfDelay: Infinity,
  });

  let totalLinesRead = 0;
  let successCount = 0;
  let failureCount = 0;
  let isHeader = true;
  let batch: object[] = [];

  console.log(`Starting bulk ingestion (Batches of ${BATCH_SIZE})...`);
  const startTime = Date.now();

  for await (const line of rl) {
    if (isHeader) {
      isHeader = false;
      continue;
    }

    const columns = line.split("\t");
    if (columns.length < 9) continue;

    totalLinesRead++;

    const [
      tconst,
      titleType,
      primaryTitle,
      originalTitle,
      isAdult,
      startYear,
      endYear,
      runtimeMinutes,
      genres,
    ] = columns;
    const cleanGenres = genres !== "\\N" ? genres.replace(/,/g, ", ") : "";
    const cleanYear = startYear !== "\\N" ? parseInt(startYear) : null;

    const payload = {
      id: totalLinesRead,
      imdb_id: tconst,
      title: primaryTitle !== "\\N" ? primaryTitle : "",
      type: titleType !== "\\N" ? titleType : "",
      genres: cleanGenres,
      release_year: cleanYear,
    };

    batch.push(payload);

    if (batch.length === BATCH_SIZE) {
      try {
        const count = await sendBatch(batch);
        successCount += count;
      } catch {
        failureCount += batch.length;
        if (failureCount === BATCH_SIZE) {
          console.error("The server rejected a bulk write.");
        }
      }
      batch = [];

      if (successCount % 1000 === 0) {
        const elapsedMin = ((Date.now() - startTime) / 1000 / 60).toFixed(2);
        console.log(
          `⚡ Speedometer: ${successCount} indexed. (Read: ${totalLinesRead} lines, Elapsed: ${elapsedMin} min)`,
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

  const totalTimeMin = ((Date.now() - startTime) / 1000 / 60).toFixed(2);
  console.log(`\n--- Batch Indexing Complete ---`);
  console.log(`Execution Time:     ${totalTimeMin} minutes`);
  console.log(`Total Rows Parsed:  ${totalLinesRead}`);
  console.log(`Engine Accepted:    ${successCount}`);
  console.log(`Engine Rejected:    ${failureCount}`);
}

async function main() {
  await createCollection();
  await indexImdbTitles();
}

main();
