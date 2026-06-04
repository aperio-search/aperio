import fs from "node:fs";
import http from "node:http";
import readline from "node:readline";

const COLLECTION_NAME = "imdb_titles";
const DATA_FILE_PATH = "./title.basics.tsv";
const CONCURRENCY_LIMIT = 500;
const API_SECRET = "SecretApiKey";

const agent = new http.Agent({
  keepAlive: true,
  maxSockets: CONCURRENCY_LIMIT,
});

const headers = {
  "Content-Type": "application/json",
  Authorization: API_SECRET,
};

async function apiPost(
  path: string,
  body: object,
): Promise<{ status: number }> {
  return await new Promise((resolve, reject) => {
    const payload = JSON.stringify(body);
    const options: http.RequestOptions = {
      hostname: "localhost",
      port: 3000,
      path,
      method: "POST",
      headers: {
        ...headers,
        "Content-Length": Buffer.byteLength(payload).toString(),
      },
      agent,
    };

    const req = http.request(options, (res) => {
      res.resume();
      resolve({ status: res.statusCode ?? 0 });
    });

    req.on("error", reject);
    req.write(payload);
    req.end();
  });
}

async function createCollection() {
  try {
    console.log(`Setting up collection: "${COLLECTION_NAME}"...`);
    const res = await apiPost("/collections", {
      name: COLLECTION_NAME,
      id_type: "number",
      searchable_fields: ["title", "type", "genres"],
    });
    if (res.status === 409) {
      console.log("Collection already exists. Moving to indexing...");
    } else if (res.status === 401) {
      console.error('Authentication failed! Check your "SecretApiKey".');
      process.exit(1);
    } else if (res.status < 200 || res.status >= 300) {
      throw new Error(`HTTP ${res.status}`);
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

    const requestPromise = apiPost(
      `/collections/${COLLECTION_NAME}/items`,
      payload,
    )
      .then((res) => {
        if (res.status >= 200 && res.status < 300) {
          successCount++;
        } else {
          failureCount++;
          if (failureCount === 1 && res.status === 401) {
            console.error(
              "The server rejected an item write with a 401 Unauthorized status.",
            );
          }
          if (failureCount % 20000 === 0) {
            console.error(`Ingestion error batch sample: HTTP ${res.status}`);
          }
        }
      })
      .catch(() => {
        failureCount++;
        if (failureCount === 1) {
          console.error("Network error on item write.");
        }
        if (failureCount % 20000 === 0) {
          console.error(`Ingestion error batch sample: network error`);
        }
      });

    currentBatch.push(requestPromise);

    // Execute batch once limit is met
    if (currentBatch.length === CONCURRENCY_LIMIT) {
      await Promise.all(currentBatch);
      currentBatch = [];

      if (successCount % 1000 === 0) {
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
