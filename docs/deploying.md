# Deploying

## Docker Configuration

To ensure your Aperio automatically recovers from system reboots, always pass the `--restart always` flag to Docker:

```sh
docker run -e DATA_DIR=/data --restart-always -d -p 3000:3000 --name aperio ghcr.io/aperio-search/aperio
```

## Storage Recommendations

Aperio is heavily optimized for low-latency I/O operations. The underlying storage hardware directly impacts search and indexing speeds.

For the best performance, always run Aperio on instances with local, physically attached SSDs. If deploying on cloud, opt for instance types that feature local NVMe drives rather than standard network volumes for your DATA_DIR.

## Memory Usage

Aperio uses LMDB for its database engine. By default, LMDB will aggressively use available system memory for its memory-mapped cache. This is intentional as it maximizes search and indexing throughput.

You can safely limit memory with Docker's `--memory` flag. Aperio runs
comfortably on as little as 256 MB:

```sh
docker run -e DATA_DIR=/data --memory=256m --restart always -d -p 3000:3000 --name aperio ghcr.io/aperio-search/aperio
```

## Happy Coding!

Thank you for using Aperio. If Aperio is helping your business or project, consider supporting its development on [GitHub Sponsors](https://github.com/sponsors/andresribeiro) to keep it fast, lean, and actively maintained.
