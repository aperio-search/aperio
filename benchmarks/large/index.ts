import fs from "node:fs";
import readline from "node:readline";

// --- CONFIGURATION ---
const BASE_URL = "http://localhost:3000";
const COLLECTION_NAME = "imdb_titles";
const DATA_FILE_PATH = "./title.basics.tsv";
const CONCURRENCY_LIMIT = 500;
const API_SECRET = "SecretApiKey";

const headers = {
  "Content-Type": "application/json",
  Authorization: API_SECRET,
};

async function createCollection() {
  try {
    console.log(`Setting up collection: "${COLLECTION_NAME}"...`);
    const response = await fetch(`${BASE_URL}/collections`, {
      method: "POST",
      headers,
      body: JSON.stringify({
        name: COLLECTION_NAME,
        id_type: "number",
        searchable_fields: ["title", "type", "genres"],
      }),
    });
    if (response.status === 409) {
      console.log("Collection already exists. Moving to indexing...");
    } else if (response.status === 401) {
      console.error('🚨 Authentication failed! Check your "SecretApiKey".');
      process.exit(1);
    } else if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    } else {
      console.log("Collection verified/ready.");
    }
  } catch (error) {
    console.error("Failed to create collection:", error.message);
    process.exit(1);
  }
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
  let currentBatch = [];

  console.log(
    `Starting rapid authorized batching (Chunks of ${CONCURRENCY_LIMIT})...`,
  );
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

    const requestPromise = fetch(
      `${BASE_URL}/collections/${COLLECTION_NAME}/items`,
      {
        method: "POST",
        headers,
        body: JSON.stringify(payload),
      },
    )
      .then((res) => {
        if (res.ok) {
          successCount++;
        } else {
          failureCount++;
          if (failureCount === 1 && res.status === 401) {
            console.error(
              "🚨 The server rejected an item write with a 401 Unauthorized status.",
            );
          }
          if (failureCount % 20000 === 0) {
            console.error(
              `🚨 Ingestion error batch sample: HTTP ${res.status}`,
            );
          }
        }
      })
      .catch((err) => {
        failureCount++;
        if (failureCount === 1) {
          console.error("🚨 Network error on item write.");
        }
        if (failureCount % 20000 === 0) {
          console.error(`🚨 Ingestion error batch sample: ${err.message}`);
        }
      });

    currentBatch.push(requestPromise);

    // Execute batch once limit is met
    if (currentBatch.length === CONCURRENCY_LIMIT) {
      await Promise.all(currentBatch);
      currentBatch = [];

      if (successCount % 50000 === 0) {
        const elapsedMin = ((Date.now() - startTime) / 1000 / 60).toFixed(2);
        console.log(
          `⚡ Speedometer: ${successCount} indexed. (Read: ${totalLinesRead} lines, Elapsed: ${elapsedMin} min)`,
        );
      }
    }
  }

  // Process leftover lines
  if (currentBatch.length > 0) {
    await Promise.all(currentBatch);
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
