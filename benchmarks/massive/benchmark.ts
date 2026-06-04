const BASE_URL = "http://localhost:3000";
const COLLECTION_NAME = "wikipedia";
const API_SECRET = "SecretApiKey";
const DURATION_MS = 5000;
const TARGET_QPS = 50;
const MAX_CONCURRENT_REQUESTS = 100;
const REQUEST_TIMEOUT_MS = 2000;

const BIP39_URL =
  "https://raw.githubusercontent.com/bitcoin/bips/master/bip-0039/english.txt";

interface SearchResponse {
  elapsed_ms?: number;
}

interface BenchmarkMetrics {
  totalFired: number;
  completed: number;
  failed: number;
  serverElapsedTimes: number[];
}

let bip39Words: string[] = [];

async function initializeWordlist(): Promise<void> {
  console.log("Fetching BIP39 wordlist from GitHub...");
  try {
    const response = await fetch(BIP39_URL);
    if (!response.ok) throw new Error(`HTTP ${response.status}`);

    const text = await response.text();
    bip39Words = text
      .split("\n")
      .map((word) => word.trim())
      .filter(Boolean);
    console.log(
      `Successfully loaded ${bip39Words.length} cache-busting words.\n`,
    );
  } catch {
    console.error(
      "Failed to load BIP39 wordlist. Falling back to basic array.",
    );
    bip39Words = [
      "matrix",
      "star",
      "dark",
      "knight",
      "inception",
      "action",
      "drama",
      "space",
    ];
  }
}

function getRandomBip39Query(): string {
  const word1 = bip39Words[Math.floor(Math.random() * bip39Words.length)];
  const word2 = bip39Words[Math.floor(Math.random() * bip39Words.length)];
  return `${word1} ${word2}`;
}

async function fetchWithTimeout(
  url: string,
  options: RequestInit & { timeout?: number },
): Promise<Response> {
  const { timeout = REQUEST_TIMEOUT_MS, ...fetchOptions } = options;

  const controller = new AbortController();
  const id = setTimeout(() => controller.abort(), timeout);

  try {
    const response = await fetch(url, {
      ...fetchOptions,
      signal: controller.signal,
    });
    clearTimeout(id);
    return response;
  } catch (error) {
    clearTimeout(id);
    throw error;
  }
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function runConstantQpsBenchmark(): Promise<void> {
  await initializeWordlist();

  console.log(`==================================================`);
  console.log(`Running Target QPS Benchmark (Native Fetch)`);
  console.log(`Target Rate:       ${TARGET_QPS} QPS`);
  console.log(`Duration:          ${DURATION_MS / 1000} seconds`);
  console.log(`Expected Requests: ${TARGET_QPS * (DURATION_MS / 1000)}`);
  console.log(`Collection:        ${COLLECTION_NAME}`);
  console.log(`==================================================\n`);

  const metrics: BenchmarkMetrics = {
    totalFired: 0,
    completed: 0,
    failed: 0,
    serverElapsedTimes: [],
  };

  const startTime = Date.now();
  const activePromises: Set<Promise<void>> = new Set();

  const intervalMs = 1000 / TARGET_QPS;

  while (Date.now() - startTime < DURATION_MS) {
    const requestStartTime = Date.now();

    if (activePromises.size >= MAX_CONCURRENT_REQUESTS) {
      await Promise.race(activePromises);
    }

    metrics.totalFired++;
    const term = getRandomBip39Query();
    const targetUrl = `${BASE_URL}/collections/${COLLECTION_NAME}/search?q=${encodeURIComponent(term)}&take=20`;

    const requestPromise = fetchWithTimeout(targetUrl, {
      method: "GET",
      headers: { Authorization: API_SECRET },
    })
      .then(async (response) => {
        if (!response.ok) throw new Error();

        const data = (await response.json()) as SearchResponse;
        metrics.completed++;

        const engineMs = data?.elapsed_ms;
        if (typeof engineMs === "number") {
          metrics.serverElapsedTimes.push(engineMs);
        }
      })
      .catch(() => {
        metrics.failed++;
      });

    activePromises.add(requestPromise);
    requestPromise.finally(() => activePromises.delete(requestPromise));

    const elapsedTick = Date.now() - requestStartTime;
    const timeToWait = intervalMs - elapsedTick;
    if (timeToWait > 0) {
      await sleep(timeToWait);
    }
  }

  await Promise.all(activePromises);

  const actualDurationMs = Date.now() - startTime;
  printReport(metrics, actualDurationMs);
}

function printReport(metrics: BenchmarkMetrics, totalTimeMs: number): void {
  const { serverElapsedTimes, completed, failed, totalFired } = metrics;

  if (serverElapsedTimes.length === 0) {
    console.error("All queries failed or engine returned no metrics.");
    return;
  }

  const sum = serverElapsedTimes.reduce((a, b) => a + b, 0);
  const avg = (sum / serverElapsedTimes.length).toFixed(3);

  serverElapsedTimes.sort((a, b) => a - b);
  const max = Math.max(...serverElapsedTimes).toFixed(3);
  const p95 =
    serverElapsedTimes[Math.floor(serverElapsedTimes.length * 0.95)].toFixed(3);
  const p99 =
    serverElapsedTimes[Math.floor(serverElapsedTimes.length * 0.99)].toFixed(3);

  const actualQps = ((totalFired / totalTimeMs) * 1000).toFixed(2);
  const successQps = ((completed / totalTimeMs) * 1000).toFixed(2);

  console.log(`Load Test Results (${TARGET_QPS} Target QPS)`);
  console.log(`--------------------------------------------------`);
  console.log(`Total Fired Requests:     ${totalFired}`);
  console.log(`Successful Responses:    ${completed}`);
  console.log(`Failed/Dropped Requests:  ${failed}`);
  console.log(`Actual Attempted Rate:    ${actualQps} QPS`);
  console.log(`Successful Throughput:    ${successQps} QPS`);
  console.log(
    `Actual Test Duration:    ${(totalTimeMs / 1000).toFixed(2)} seconds`,
  );
  console.log(``);
  console.log(`Engine Internal Search Latency (Server-Side Only):`);
  console.log(`  Average Latency:        ${avg} ms`);
  console.log(`  p95 Latency (95%):      ${p95} ms`);
  console.log(`  p99 Latency (99%):      ${p99} ms`);
  console.log(`  Maximum Latency:        ${max} ms`);
  console.log(`==================================================`);
}

runConstantQpsBenchmark();
