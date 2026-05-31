const fs = require("fs");
const readline = require("readline");
const axios = require("axios");
const http = require("http");

// --- CONFIGURATION ---
const BASE_URL = "http://localhost:3000";
const COLLECTION_NAME = "imdb_titles";
const DATA_FILE_PATH = "./title.basics.tsv";
const CONCURRENCY_LIMIT = 500; // Controlled Promise.all batch size
const API_SECRET = "SecretApiKey"; // Your authorization token

// Universal Axios instance with authorization preset
const client = axios.create({
  baseURL: BASE_URL,
  headers: {
    "Content-Type": "application/json",
    Authorization: API_SECRET, // Authenticates all traffic going to your search engine
  },
  httpAgent: new http.Agent({ keepAlive: true, maxSockets: CONCURRENCY_LIMIT }),
  timeout: 60000,
});

async function createCollection() {
  try {
    console.log(`Setting up collection: "${COLLECTION_NAME}"...`);
    await client.post("/collections", {
      name: COLLECTION_NAME,
      id_type: "number",
      searchable_fields: ["title", "type", "genres"],
    });
    console.log("Collection verified/ready.");
  } catch (error) {
    if (error.response && error.response.status === 409) {
      console.log("Collection already exists. Moving to indexing...");
    } else if (error.response && error.response.status === 401) {
      console.error('🚨 Authentication failed! Check your "SecretApiKey".');
      process.exit(1);
    } else {
      console.error("Failed to create collection:", error.message);
      process.exit(1);
    }
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

    // Construct individual insertion request
    const requestPromise = client
      .post(`/collections/${COLLECTION_NAME}/items`, payload)
      .then(() => {
        successCount++;
      })
      .catch((err) => {
        failureCount++;
        if (failureCount === 1 && err.response?.status === 401) {
          console.error(
            "🚨 The server rejected an item write with a 401 Unauthorized status.",
          );
        }
        if (failureCount % 20000 === 0) {
          console.error(
            `🚨 Ingestion error batch sample: ${err.message} (Status: ${err.response?.status || "network_error"})`,
          );
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
