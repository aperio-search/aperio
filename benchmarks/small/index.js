import fs from "node:fs";
import readline from "node:readline";

const BASE_URL = "http://localhost:3000";
const COLLECTION_NAME = "books";
const DATA_FILE_PATH = "./books.csv";
const CONCURRENCY_LIMIT = 500;
const API_SECRET = "SecretApiKey";

const headers = {
  "Content-Type": "application/json",
  Authorization: API_SECRET,
};

async function apiPost(path, body) {
  const response = await fetch(`${BASE_URL}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  if (!response.ok) {
    const err = new Error(`HTTP ${response.status}`);
    err.status = response.status;
    throw err;
  }
  return response;
}

async function createCollection() {
  try {
    console.log(`Setting up collection: "${COLLECTION_NAME}"...`);
    await apiPost("/collections", {
      name: COLLECTION_NAME,
      id_type: "number",
      searchable_fields: ["title", "authors", "publisher"],
    });
    console.log("Collection verified/ready.");
  } catch (error) {
    console.error(error);
    if (error.status === 409) {
      console.log("Collection already exists. Moving to indexing...");
    } else if (error.status === 401) {
      console.error('Authentication failed! Check your "SecretApiKey".');
      process.exit(1);
    } else {
      console.error("Failed to create collection:", error.message);
      process.exit(1);
    }
  }
}

function parseCSVLine(line) {
  const result = [];
  let current = "";
  let inQuotes = false;
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

async function indexBooks() {
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

    const columns = parseCSVLine(line);
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

    const payload = {
      id: totalLinesRead,
      title: title || "",
      authors: authors || "",
      average_rating: average_rating ? parseFloat(average_rating) : null,
      isbn: isbn || "",
      isbn13: isbn13 || "",
      language_code: language_code || "",
      num_pages: num_pages ? parseInt(num_pages) : null,
      ratings_count: ratings_count ? parseInt(ratings_count) : null,
      text_reviews_count: text_reviews_count
        ? parseInt(text_reviews_count)
        : null,
      publication_date: publication_date || "",
      publisher: publisher || "",
    };

    const requestPromise = apiPost(
      `/collections/${COLLECTION_NAME}/items`,
      payload,
    )
      .then(() => {
        successCount++;
      })
      .catch((err) => {
        failureCount++;
        if (failureCount === 1 && err.status === 401) {
          console.error(
            "The server rejected an item write with a 401 Unauthorized status.",
          );
        }
        if (failureCount % 20000 === 0) {
          console.error(
            `Ingestion error batch sample: ${err.message} (Status: ${err.status || "network_error"})`,
          );
        }
      });

    currentBatch.push(requestPromise);

    if (currentBatch.length === CONCURRENCY_LIMIT) {
      await Promise.all(currentBatch);
      currentBatch = [];

      if (successCount % 50000 === 0) {
        const elapsedMin = ((Date.now() - startTime) / 1000 / 60).toFixed(2);
        console.log(
          `Speedometer: ${successCount} indexed. (Read: ${totalLinesRead} lines, Elapsed: ${elapsedMin} min)`,
        );
      }
    }
  }

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
  await indexBooks();
}

main();
