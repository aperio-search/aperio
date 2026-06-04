import cluster from "node:cluster";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import * as sax from "sax";
import wtf from "wtf_wikipedia";

const COLLECTION_NAME = "wikipedia";
const BATCH_SIZE = 300;
const API_SECRET = "SecretApiKey";

const BASE_URL = "http://localhost:3000";
const HEADERS = {
  "Content-Type": "application/json",
  Authorization: API_SECRET,
};

// --- IPC Message Types ---
type WorkerMessage =
  | { type: "REQUEST_FILE" }
  | {
      type: "FILE_DONE";
      articles: number;
      success: number;
      failure: number;
      bytes: number;
    }
  | { type: "PROGRESS"; success: number; failure: number };

type MasterMessage =
  | {
      type: "PROCESS_FILE";
      filePath: string;
      fileIndex: number;
      totalFiles: number;
      totalWorkers: number;
    }
  | { type: "NO_MORE_FILES" };

// --- Shared Helper Functions ---
async function apiPost(p: string, body: object): Promise<number> {
  const res = await fetch(`${BASE_URL}${p}`, {
    method: "POST",
    headers: HEADERS,
    body: JSON.stringify(body),
  });
  return res.status;
}

function cleanBody(rawWikiText: string): string {
  try {
    let text = wtf(rawWikiText).text();
    text = text.replace(/\n+/g, " ");
    text = text.replace(/\s+/g, " ").trim();
    return text;
  } catch {
    return "";
  }
}

// ============================================================================
// MASTER PROCESS
// ============================================================================
async function createCollection() {
  try {
    console.log(`Setting up collection: "${COLLECTION_NAME}"...`);
    const status = await apiPost("/collections", {
      name: COLLECTION_NAME,
      id_type: "number", // Reverted back to number
      searchable_fields: ["title", "body"],
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

async function masterProcess() {
  await createCollection();

  const dir = __dirname;
  const files = fs
    .readdirSync(dir)
    .filter((f) => f.endsWith(".xml"))
    .sort();

  if (files.length === 0) {
    console.log("No .xml files found in", dir);
    return;
  }

  const numWorkers = Math.max(
    1,
    (os.availableParallelism?.() || os.cpus().length) - 1,
  );
  console.log(
    `Found ${files.length} XML file(s). Spawning ${numWorkers} workers.\n`,
  );

  const state = {
    articles: 0,
    success: 0,
    failure: 0,
    totalBytesSent: 0,
    startTime: Date.now(),
  };

  let fileQueueIndex = 0;
  let activeWorkers = numWorkers;
  let lastSpeedometerLog = 0;

  for (let i = 0; i < numWorkers; i++) {
    const worker = cluster.fork();

    worker.on("message", (msg: WorkerMessage) => {
      if (msg.type === "REQUEST_FILE") {
        if (fileQueueIndex < files.length) {
          const filePayload: MasterMessage = {
            type: "PROCESS_FILE",
            filePath: path.join(dir, files[fileQueueIndex]),
            fileIndex: fileQueueIndex + 1,
            totalFiles: files.length,
            totalWorkers: numWorkers, // Pass down to compute step IDs
          };
          worker.send(filePayload);
          fileQueueIndex++;
        } else {
          worker.send({ type: "NO_MORE_FILES" } as MasterMessage);
        }
      }

      if (msg.type === "PROGRESS") {
        state.success += msg.success;
        state.failure += msg.failure;

        if (state.success - lastSpeedometerLog >= 1000) {
          lastSpeedometerLog = state.success - (state.success % 1000);
          const elapsedMin = (
            (Date.now() - state.startTime) /
            1000 /
            60
          ).toFixed(2);
          console.log(
            `Speedometer: ${state.success} indexed (${state.failure} failed, ${elapsedMin} min)`,
          );
        }
      }

      if (msg.type === "FILE_DONE") {
        state.articles += msg.articles;
        state.totalBytesSent += msg.bytes;
      }
    });

    worker.on("exit", () => {
      activeWorkers--;
      if (activeWorkers === 0) {
        const totalTimeMin = (
          (Date.now() - state.startTime) /
          1000 /
          60
        ).toFixed(2);
        console.log(`\n--- Bulk Indexing Complete ---`);
        console.log(`Execution Time:    ${totalTimeMin} minutes`);
        console.log(`Total Articles:    ${state.articles}`);
        console.log(`Engine Accepted:   ${state.success}`);
        console.log(`Engine Rejected:   ${state.failure}`);
        console.log(`Data Sent (Bytes): ${state.totalBytesSent}`);
      }
    });
  }
}

// ============================================================================
// WORKER PROCESS
// ============================================================================
async function sendBatch(
  batch: object[],
): Promise<{ count: number; bytes: number }> {
  const json = JSON.stringify(batch);
  const bytes = Buffer.byteLength(json, "utf8");
  const res = await fetch(
    `${BASE_URL}/collections/${COLLECTION_NAME}/items/bulk`,
    {
      method: "POST",
      headers: HEADERS,
      body: json,
    },
  );
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }
  const body = (await res.json()) as { ok: boolean; count: number };
  return { count: body.count, bytes };
}

// Global sequence accumulator per worker across ALL files it processes
let workerGlobalArticlesSeq = 0;

async function processFile(
  filePath: string,
  fileIndex: number,
  totalFiles: number,
  totalWorkers: number,
): Promise<void> {
  const fileName = path.basename(filePath);
  const workerId = cluster.worker?.id || 0;
  console.log(
    `[Worker ${workerId}][${fileIndex}/${totalFiles}] Started processing: ${fileName}`,
  );

  const fileStream = fs.createReadStream(filePath, {
    encoding: "utf8",
    highWaterMark: 65536,
  });
  const saxStream = sax.createStream(true, { lowercase: true, trim: true });

  let currentTag = "";
  let inPage = false;
  let pageTitle = "";
  let pageBody = "";
  let namespace = "";

  let batch: object[] = [];
  let fileArticleCount = 0;

  let fileSuccess = 0;
  let fileFailure = 0;
  let fileBytes = 0;

  saxStream.on("opentag", (node) => {
    currentTag = node.name;
    if (currentTag === "page") {
      inPage = true;
      pageTitle = "";
      pageBody = "";
      namespace = "";
    }
  });

  saxStream.on("text", (text) => {
    if (!inPage) return;
    switch (currentTag) {
      case "title":
        pageTitle += text;
        break;
      case "ns":
        namespace += text;
        break;
      case "text":
        pageBody += text;
        break;
    }
  });

  saxStream.on("closetag", (tagName) => {
    if (tagName === "page") {
      inPage = false;
      if (namespace === "0" && pageTitle) {
        fileArticleCount++;
        workerGlobalArticlesSeq++; // Increment persistent cross-file counter

        const clean = cleanBody(pageBody);

        // Strided math formula guarantees completely unique numerical IDs
        const uniqueNumericalId =
          workerGlobalArticlesSeq * totalWorkers + workerId;

        batch.push({
          id: uniqueNumericalId,
          title: pageTitle,
          body: clean,
        });
      }
    }
    if (currentTag === tagName) {
      currentTag = "";
    }
  });

  saxStream.on("error", (err) => {
    console.error(
      `  [Worker ${workerId}] Parse error in ${fileName}: ${err.message}`,
    );
  });

  for await (const chunk of fileStream) {
    saxStream.write(chunk);

    while (batch.length >= BATCH_SIZE) {
      const toSend = batch.splice(0, BATCH_SIZE);
      try {
        const result = await sendBatch(toSend);
        fileSuccess += result.count;
        fileBytes += result.bytes;

        process.send!({
          type: "PROGRESS",
          success: result.count,
          failure: 0,
        } as WorkerMessage);
      } catch (err) {
        fileFailure += toSend.length;
        process.send!({
          type: "PROGRESS",
          success: 0,
          failure: toSend.length,
        } as WorkerMessage);

        const e = err as Error;
        if (e.message?.includes("401")) {
          console.error(
            `[Worker ${workerId}] Server rejected write with 401 Unauthorized status.`,
          );
          process.exit(1);
        }
      }
    }
  }

  saxStream.end();

  if (batch.length > 0) {
    try {
      const result = await sendBatch(batch);
      fileSuccess += result.count;
      fileBytes += result.bytes;
      process.send!({
        type: "PROGRESS",
        success: result.count,
        failure: 0,
      } as WorkerMessage);
    } catch {
      fileFailure += batch.length;
      process.send!({
        type: "PROGRESS",
        success: 0,
        failure: batch.length,
      } as WorkerMessage);
    }
  }

  console.log(
    `[Worker ${workerId}][${fileIndex}/${totalFiles}] Done: ${fileName} (${fileArticleCount} articles)`,
  );

  process.send!({
    type: "FILE_DONE",
    articles: fileArticleCount,
    success: fileSuccess,
    failure: fileFailure,
    bytes: fileBytes,
  } as WorkerMessage);
}

function workerProcess() {
  process.on("message", async (msg: MasterMessage) => {
    if (msg.type === "PROCESS_FILE") {
      await processFile(
        msg.filePath,
        msg.fileIndex,
        msg.totalFiles,
        msg.totalWorkers,
      );
      process.send!({ type: "REQUEST_FILE" } as WorkerMessage);
    } else if (msg.type === "NO_MORE_FILES") {
      process.exit(0);
    }
  });

  process.send!({ type: "REQUEST_FILE" } as WorkerMessage);
}

// ============================================================================
// MAIN RUNNER
// ============================================================================
function main() {
  if (cluster.isPrimary) {
    masterProcess();
  } else {
    workerProcess();
  }
}

main();
