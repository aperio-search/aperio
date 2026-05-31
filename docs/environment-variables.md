# Environment Variables

Aperio is configured through environment variables for simple containerized deployment.

## Reference

| Variable | Default | Description |
|---|---|---|
| `DATA_DIR` | `data` | Directory for persistent data. Aperio creates a `{DATA_DIR}/aperio_data` subdirectory to store its database files |
| `CONFIG_FILE` | *(none)* | Path to an optional [TOML config file](/configuration). If unset, missing, or malformed, safe defaults are used with only a warning to stderr |

## Example

```sh
docker run \
  -e DATA_DIR=/data \
  -e CONFIG_FILE=/data/config.toml \
  -p 3000:3000 \
  andresribeiro/aperio
```

See the [Configuration](/configuration) page for all available TOML settings.
