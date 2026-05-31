const axios = require("axios");
const http = require("http");

// --- CONFIGURATION ---
const BASE_URL = "http://localhost:3000";
const COLLECTION_NAME = "imdb_titles";
const API_SECRET = "SecretApiKey";

// Target Arrival Rate (Queries Per Second)
const TARGET_QPS = 500; // Change this to test different load levels (e.g., 200, 500, 1000, 1500)
const DURATION_SECONDS = 5; // How long to sustain this traffic level

const SEARCH_TERMS = [
  "The",
  "Matrix",
  "Star",
  "Dark",
  "Knight",
  "Inception",
  "Action",
  "Drama",
];

// High capacity sockets to prevent Node itself from bottlenecking the test
const client = axios.create({
  baseURL: BASE_URL,
  headers: { Authorization: API_SECRET },
  httpAgent: new http.Agent({ keepAlive: true, maxSockets: TARGET_QPS * 2 }),
  timeout: 5000, // 5-second timeout if the engine chokes
});

function getRandomTerm() {
  return SEARCH_TERMS[Math.floor(Math.random() * SEARCH_TERMS.length)];
}

async function runRateBenchmark() {
  const totalTargetQueries = TARGET_QPS * DURATION_SECONDS;

  console.log(`==================================================`);
  console.log(`🎯 Running Open-Loop Rate Benchmark`);
  console.log(`👉 Target Throughput:  ${TARGET_QPS} Queries / Second`);
  console.log(`👉 Test Duration:     ${DURATION_SECONDS} seconds`);
  console.log(`👉 Total Payload:     ${totalTargetQueries} queries`);
  console.log(`==================================================\n`);

  const serverElapsedTimes = [];
  let firedCount = 0;
  let completedCount = 0;
  let failedCount = 0;

  const startTime = Date.now();

  // High-precision interval firing loop
  const intervalMs = 1000 / TARGET_QPS;

  return new Promise((resolve) => {
    const timer = setInterval(async () => {
      if (firedCount >= totalTargetQueries) {
        clearInterval(timer);
        return;
      }

      firedCount++;
      const term = getRandomTerm();

      // Fire and Forget immediately to maintain target QPS, handling metrics inside the promise chain
      client
        .get(`/collections/${COLLECTION_NAME}/search`, {
          params: { q: term, take: 20 },
        })
        .then((response) => {
          completedCount++;
          const engineMs = response.data?.elapsed_ms;
          if (typeof engineMs === "number") {
            serverElapsedTimes.push(engineMs);
          }
          checkIfFinished();
        })
        .catch(() => {
          failedCount++;
          checkIfFinished();
        });
    }, intervalMs);

    function checkIfFinished() {
      // Once all fired requests have settled (either resolved or rejected)
      if (completedCount + failedCount === totalTargetQueries) {
        const wallClockTimeMs = Date.now() - startTime;
        printReport(
          serverElapsedTimes,
          completedCount,
          failedCount,
          wallClockTimeMs,
        );
        resolve();
      }
    }
  });
}

function printReport(times, completed, failed, totalTimeMs) {
  if (times.length === 0) {
    console.error("❌ All queries failed or engine returned no metrics.");
    return;
  }

  const sum = times.reduce((a, b) => a + b, 0);
  const avg = (sum / times.length).toFixed(3);
  const max = Math.max(...times).toFixed(3);

  times.sort((a, b) => a - b);
  const p95 = times[Math.floor(times.length * 0.95)].toFixed(3);
  const p99 = times[Math.floor(times.length * 0.99)].toFixed(3);

  const actualQps = ((completed / totalTimeMs) * 1000).toFixed(2);

  console.log(`📊 Load Test Results @ ${TARGET_QPS} Target QPS`);
  console.log(`--------------------------------------------------`);
  console.log(`Fired Queries:            ${completed + failed}`);
  console.log(`Engine Successful:        ${completed}`);
  console.log(`Engine Failed/Dropped:    ${failed}`);
  console.log(`Actual Sustained QPS:     ${actualQps} qps`);
  console.log(``);
  console.log(`⏱️ Engine Internal Search Latency (Server-Side Only):`);
  console.log(`  Average Latency:        ${avg} ms`);
  console.log(`  p95 Latency (95%):      ${p95} ms`);
  console.log(`  p99 Latency (99%):      ${p99} ms`);
  console.log(`  Maximum Latency:        ${max} ms`);
  console.log(`==================================================`);
}

runRateBenchmark();
