const BASE_URL = "http://localhost:3000";
const COLLECTION_NAME = "imdb_titles";
const API_SECRET = "SecretApiKey";

const TARGET_QPS = 500;
const DURATION_SECONDS = 5;

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

const headers = { Authorization: API_SECRET };

function getRandomTerm() {
  return SEARCH_TERMS[Math.floor(Math.random() * SEARCH_TERMS.length)];
}

async function apiGet(path: string) {
  const response = await fetch(`${BASE_URL}${path}`, { headers });
  if (!response.ok) {
    const err = new Error(`HTTP ${response.status}`);
    err.status = response.status;
    throw err;
  }
  return response.json();
}

async function runRateBenchmark() {
  const totalTargetQueries = TARGET_QPS * DURATION_SECONDS;

  console.log(`==================================================`);
  console.log(`Running Open-Loop Rate Benchmark`);
  console.log(`Target Throughput:  ${TARGET_QPS} Queries / Second`);
  console.log(`Test Duration:     ${DURATION_SECONDS} seconds`);
  console.log(`Total Payload:     ${totalTargetQueries} queries`);
  console.log(`==================================================\n`);

  const serverElapsedTimes: number[] = [];
  let firedCount = 0;
  let completedCount = 0;
  let failedCount = 0;

  const startTime = Date.now();

  const intervalMs = 1000 / TARGET_QPS;

  return new Promise((resolve) => {
    const timer = setInterval(async () => {
      if (firedCount >= totalTargetQueries) {
        clearInterval(timer);
        return;
      }

      firedCount++;
      const term = getRandomTerm();

      apiGet(
        `/collections/${COLLECTION_NAME}/search?q=${encodeURIComponent(term)}&take=20`,
      )
        .then((data) => {
          completedCount++;
          const engineMs = data?.elapsed_ms;
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

function printReport(
  times: any[],
  completed: number,
  failed: number,
  totalTimeMs: number,
) {
  if (times.length === 0) {
    console.error("All queries failed or engine returned no metrics.");
    return;
  }

  const sum = times.reduce((a, b) => a + b, 0);
  const avg = (sum / times.length).toFixed(3);
  const max = Math.max(...times).toFixed(3);

  times.sort((a, b) => a - b);
  const p95 = times[Math.floor(times.length * 0.95)].toFixed(3);
  const p99 = times[Math.floor(times.length * 0.99)].toFixed(3);

  const actualQps = ((completed / totalTimeMs) * 1000).toFixed(2);

  console.log(`Load Test Results @ ${TARGET_QPS} Target QPS`);
  console.log(`--------------------------------------------------`);
  console.log(`Fired Queries:            ${completed + failed}`);
  console.log(`Engine Successful:        ${completed}`);
  console.log(`Engine Failed/Dropped:    ${failed}`);
  console.log(`Actual Sustained QPS:     ${actualQps} qps`);
  console.log(``);
  console.log(`Engine Internal Search Latency (Server-Side Only):`);
  console.log(`  Average Latency:        ${avg} ms`);
  console.log(`  p95 Latency (95%):      ${p95} ms`);
  console.log(`  p99 Latency (99%):      ${p99} ms`);
  console.log(`  Maximum Latency:        ${max} ms`);
  console.log(`==================================================`);
}

runRateBenchmark();
