# Benchmarks

The benchmarks are easy and fast to be reproducible (less than 1 hour, most of the time spent on indexing).

## Datasets

- Small: [Books](https://github.com/MainakRepositor/Datasets/blob/master/books.csv) - 11,127 items (1.5MB)
- Large: [IMDb Titles](https://datasets.imdbws.com) - 12,533,197 items (1.1GB)

## Reproducing

This test was performed on 2 Hetzner servers:

- `Client` (CCX23 - Dedicated vCPU): 4 vCPUs, 16GB RAM, 160GB NVMe SSD
- `Worker` (Bare Metal from Server Auction): AMD EPYC 7401P, 2 x RAM 32768 MB DDR5, 2 x SSD M.2 NVMe 1 TB, 2 x SSD SATA 1,92 TB Datacenter, NIC 1 Gbit - Intel i225-LM

Both running Debian 13. The amount of vCPU and RAM available on the cloud servers would be sufficient. However, since our focus is on I/O, the test could be compromised by a noisy neighbor saturating the SSD.

### Worker Configuration

```bash
apt update
curl -fsSL https://get.docker.com -o get-docker.sh
sudo sh ./get-docker.sh

docker pull ghcr.io/aperio-search/aperio
docker run --rm -p 3000:3000 -v "$(pwd)/data:/data" --name aperio ghcr.io/aperio-search/aperio
```

### Client Configuration

```bash
apt update
apt install unzip
curl -fsSL https://bun.sh/install | bash
source /root/.bashrc

# Disable THP on the server. This prevents fjall block cache allocation to gets mapped even though the cache is logically empty
echo never | sudo tee /sys/kernel/mm/transparent_hugepage/enabled

# If you wanna the small dataset:
curl -o books.csv https://raw.githubusercontent.com/MainakRepositor/Datasets/refs/heads/master/books.csv

# If you wanna the large dataset:
curl -o title.basics.tsv.gz https://datasets.imdbws.com/title.basics.tsv.gz
gunzip title.basics.tsv.gz

touch index.ts benchmark.ts
```

- Small Dataset: copy [small/index.ts](/small/index.ts) code below into `index.ts` and [small/benchmark.ts](/small/benchmark.ts) into `benchmark.ts`. On both files replace `http://localhost:3000` with the IP of Worker.
- Large Dataset: copy [large/index.ts](/large/index.ts) code below into `index.ts` and [large/benchmark.ts](/large/benchmark.ts) into `benchmark.ts`. On both files replace `http://localhost:3000` with the IP of Worker.

### Benchmarking

Run this on Client:

```js
// Index the dataset.
bun index.ts
// The search engine indexes data asynchronously. Wait a few minutes after the script stop running.
bun benchmark.ts
```
