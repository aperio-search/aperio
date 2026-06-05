# Benchmarks

## Datasets

| Dataset | Items | Size |
|---|---|---|
| Small — [Books](https://github.com/MainakRepositor/Datasets/blob/master/books.csv) | 11,127 | 1.5 MB |
| Medium — [IMDb Titles](https://datasets.imdbws.com) | 12,533,197 | 1.1 GB |

## Results

On the benchmarks, RAM was limited to 256 MB.

| Dataset | Avg Latency | p95 | p99 | Max Latency |
|---|---|---|---|---|
| Books | 0.011 ms | 0.016 ms | 0.021 ms | 1.114 ms |
| IMDb | 0.060 ms | 0.113 ms | 0.547 ms | 8.042 ms |

## Reproducing

This test was performed on 2 Hetzner servers:

- `Worker` (Bare Metal from Server Auction): AMD Ryzen 7 7700, 2x RAM 32768 MB DDR5, 2 x SSD M.2 NVMe 1 TB, NIC 1 Gbit - Intel i225-LM
- `Client` (CCX13 - Dedicated vCPU): 2 vCPUs, 8GB RAM, 80GB NVMe SSD

Both running Debian 13.

### Worker Configuration

Aperio doesn't provide compiled binaries. You must compile it yourself.

```bash
# Turn off THP system-wide (Temporarily)
echo never > /sys/kernel/mm/transparent_hugepage/enabled
# Start aperio
./aperio
```

### Client Configuration

```bash
# Install Bun
apt update
apt install unzip
curl -fsSL https://bun.sh/install | bash
source /root/.bashrc

# If you wanna the small dataset:
curl -o books.csv https://raw.githubusercontent.com/MainakRepositor/Datasets/refs/heads/master/books.csv

# If you wanna the medium dataset:
curl -o title.basics.tsv.gz https://datasets.imdbws.com/title.basics.tsv.gz
gunzip title.basics.tsv.gz

touch index.ts benchmark.ts
```

- Small Dataset: copy [small/index.ts](/small/index.ts) code below into `index.ts` and [small/benchmark.ts](/small/benchmark.ts) into `benchmark.ts`. On both files replace `localhost` with the IP of Worker.
- Medium Dataset: copy [medium/index.ts](/medium/index.ts) code below into `index.ts` and [medium/benchmark.ts](/medium/benchmark.ts) into `benchmark.ts`. On both files replace `localhost` with the IP of Worker.

### Indexing Data

On Client:

```bash
# Index the dataset.
bun index.ts
# Make sure the indexing queue is on 0 before benchmarking
curl "http://[worker_ip]:3000/queue" -H "Authorization: SecretApiKey"
```

### Benchmarking

Stop the current process on Worker and start again:

```bash
systemd-run --scope -p MemoryMin=128M -p MemoryHigh=200M -p MemoryMax=256M -p MemorySwapMax=0 ./aperio
```

To maximize indexing performance, RAM limits are omitted during indexing phase. If you choose to constrain memory during indexing, performance will scale down accordingly.

On Client:

```bash
# Run the benchmark
bun benchmark.ts
```
