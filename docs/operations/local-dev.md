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
| 7003 – 7007 | demo inventory / payments / notifications / catalog / auth |
| 7010 | demo payment provider stand-in (not instrumented) |

The infrastructure ports sit outside the default ranges, so they don't clash with a local
Postgres, ClickHouse or Kafka. See ADR-0006.

## Generated local state (`.controlhub/`, gitignored)

- `settings.env`: your own settings, created once with defaults and never overwritten. Holds the
  ClickHouse retention in days (`CONTROLHUB_LOGS_RETENTION_DAYS`, `CONTROLHUB_SPANS_RETENTION_DAYS`,
  `CONTROLHUB_EVENTS_RETENTION_DAYS`). The services read it before `services.env`; real environment
  variables win. See "Changing retention" below.
- `infra.env`: infrastructure passwords. It is kept across bootstraps.
- `services.env`: a least-privilege connection string for each service, including the executor.
  `CONTROLHUB_ENV=dev`.
- `demo.env`: the demo services' ingestion token and endpoint.
- `local-credentials.json`: the web login plus the ingestion token. Every bootstrap generates new
  ones.
- `bin/`: copies of the service binaries that `just dev` runs. On Windows this keeps the build
  output unlocked while the services run.
- `run/`: PID files of the running `just dev`/`just demo` processes, used by `just stop` and to
  refuse a second start.

## Lifecycle

```
just bootstrap   # idempotent
just dev         # Ctrl+C stops everything
just demo
just stop        # stop leftovers of `just dev`/`just demo` (e.g. after closing their terminal)
just down        # stop containers, keep data
just reset       # asks for confirmation, then deletes volumes (all local telemetry and metadata)
```

## Changing retention

1. Edit the days in `.controlhub/settings.env`.
2. `just bootstrap` sets them as the ClickHouse table TTLs. If a shorter value would delete stored
   rows, bootstrap stops and says how many. Confirm with `just bootstrap --shrink-retention`.
3. Restart `just dev`.

Until bootstrap has applied the change, the API and the archiver refuse to start with
"`CONTROLHUB_…_RETENTION_DAYS` is N days but ClickHouse keeps … for M days; run `just bootstrap`".
The API reads logs older than one day less than the log retention from the archive
(`CONTROLHUB_ARCHIVE_HOT_WINDOW_HOURS` overrides this and must stay below the retention). See ADR-0016.

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
- **An approved action stays `approved`.** The executor is not running (it is started by
  `just dev`; look for `controlhub-executor running`). After 10 minutes the plan expires and is
  never run.
- **An action ends as `failed` with "fault control returned 404".** The demo was started without
  fault injection. `just demo` sets `CONTROLHUB_DEMO_FAULTS=1`.
- **An action is denied by "rate limit".** At most 30 plans per environment per hour. Repeated test
  runs count too (the actions integration test creates 6, the e2e 1), so wait for the window to
  pass. `just bootstrap` does not delete plans.
- **`verification_failed` after clearing faults on `gateway`.** Verification looks at the real
  error ratio, so errors propagated from `orders` still count. Clear `orders` first.
