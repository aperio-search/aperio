# Environment Variables

Aperio is configured through environment variables for simple containerized deployment.

## Reference

| Variable | Default | Description |
|---|---|---|
| `DATA_DIR` | `data` | Directory for persistent data. Aperio creates a `{DATA_DIR}/aperio_data` subdirectory to store its database files |
| `CONFIG_FILE` | *(none)* | Path to an optional [TOML config file](/configuration). If unset, missing, or malformed, safe defaults are used with only a warning to stderr |
| `MAIN_API_KEY` | `SecretApiKey` | Main API key with full access to all endpoints. Overrides the config file value if set |
| `SEARCH_API_KEY` | `PublicApiKey` | Search-only API key for `search` and `suggest` endpoints. Overrides the config file value if set |

## Example

```sh
docker run \
  -e DATA_DIR=/data \
  -e CONFIG_FILE=/data/config.toml \
  -e MAIN_API_KEY=my-secret-key \
  -e SEARCH_API_KEY=my-search-key \
  -p 3000:3000 \
  andresribeiro/aperio
```

See the [Configuration](/configuration) page for all available TOML settings.
