# Local development and operations

## Ports (all bound to 127.0.0.1)

| Port | Service |
|---|---|
| 15432 | PostgreSQL (ControlHub metadata) |
| 18123 | ClickHouse HTTP |
| 19092 | Kafka (single-node KRaft) |
| 15433 | Demo Postgres (profile `demo`) |
| 4318 / 4317 | ingest OTLP HTTP / gRPC |
| 9464 | ingest admin: `/metrics` (health: `GET :4318/health`) |
| 9465 | processor admin: `/metrics` |
| 8080 | API |
| 3000 | web |
| 7001 / 7002 | demo gateway / orders |

The infrastructure ports sit outside the default ranges, so they don't clash with a local
Postgres, ClickHouse or Kafka. See ADR-0006.

## Generated local state (`.controlhub/`, gitignored)

- `infra.env`: infrastructure passwords. It is kept across bootstraps.
- `services.env`: a least-privilege connection string for each service. `CONTROLHUB_ENV=dev`.
- `demo.env`: the demo services' ingestion token and endpoint.
- `local-credentials.json`: the web login plus the ingestion token. Every bootstrap generates new
  ones.
- `bin/`: copies of the service binaries that `just dev` runs. On Windows this keeps the build
  output unlocked while the services run.

## Lifecycle

```
just bootstrap   # idempotent
just dev         # Ctrl+C stops everything
just demo
just down        # stop containers, keep data
just reset       # asks for confirmation, then deletes volumes (all local telemetry and metadata)
```

## Health and metrics

```
curl http://127.0.0.1:9464/metrics   # ingest: accepted/rejected/failed_broker per signal, in-flight
curl http://127.0.0.1:9465/metrics   # processor: rows written, lag, processing errors
curl http://127.0.0.1:8080/api/v1/ready
```

## Troubleshooting

- **`Access is denied` while building on Windows.** An older service binary is still running.
  Stop `just dev`. The dev script already runs copies from `.controlhub/bin` to avoid this.
- **ingest returns 503.** Kafka is down or not acknowledging. This is intentional: clients must
  retry. Check `docker compose -p controlhub ps`.
- **The workspace shows an engine error, or the web build fails on `generated/wasm-manifest`.**
  Run `just wasm`, then restart the web app.
- **The map is empty.** Check that `just demo` is running and that ingest `accepted` is increasing.
  The completeness badge shows how current the data is.
