# Deploying

## 🐋 Docker Configuartion

To ensure your Aster automatically recovers from system reboots, always pass the `--restart always` flag to Docker:

```sh
docker run -e DATA_DIR=/data --restart-always -d -p 3000:3000 --name aster andresribeiro/aster
```

## 💾 Storage Recommendations

Aster is heavily optimized for low-latency I/O operations. The underlying storage hardware directly impacts search and indexing speeds.

For the best performance, always run Aster on instances with local, physically attached SSDs. If deploying on cloud, opt for instance types that feature local NVMe drives rather than standard network volumes for your DATA_DIR.

## 🔧 Happy Coding!

Thank you for using Aster. If Aster is helping your business or project, consider supporting its development on [GitHub Sponsors](https://github.com/sponsors/andresribeiro) to keep it fast, lean, and actively maintained.
