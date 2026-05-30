# Deploying

## Docker Configuration

To ensure your Aperio automatically recovers from system reboots, always pass the `--restart always` flag to Docker:

```sh
docker run -e DATA_DIR=/data --restart-always -d -p 3000:3000 --name aperio andresribeiro/aperio
```

## Storage Recommendations

Aperio is heavily optimized for low-latency I/O operations. The underlying storage hardware directly impacts search and indexing speeds.

For the best performance, always run Aperio on instances with local, physically attached SSDs. If deploying on cloud, opt for instance types that feature local NVMe drives rather than standard network volumes for your DATA_DIR.

## Happy Coding!

Thank you for using Aperio. If Aperio is helping your business or project, consider supporting its development on [GitHub Sponsors](https://github.com/sponsors/andresribeiro) to keep it fast, lean, and actively maintained.
