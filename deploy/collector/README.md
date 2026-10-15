# Collecting machine and PostgreSQL metrics

The default Compose service collects host and container metrics. Set `KARBIRD_OTLP_ENDPOINT`,
`KARBIRD_INGEST_TOKEN` (a token allowed to send metrics) and `KARBIRD_HOST_NAME`, then run
`docker compose -f deploy/collector/compose.yaml up -d collector` on the observed machine.

For PostgreSQL, create a dedicated monitoring login. Grant only access needed for statistics:

```sql
CREATE ROLE karbird_monitor LOGIN PASSWORD 'replace-with-a-generated-secret';
GRANT CONNECT ON DATABASE your_database TO karbird_monitor;
GRANT pg_read_all_stats TO karbird_monitor;
```

Store its password outside the repository. Set `KARBIRD_POSTGRES_ENDPOINT` (`host:5432`),
`KARBIRD_POSTGRES_DATABASE`, `KARBIRD_POSTGRES_USER`, and `KARBIRD_POSTGRES_PASSWORD` in the
Collector's environment. Start the optional profile with
`docker compose -f deploy/collector/compose.yaml --profile postgres up -d postgres`.
The PostgreSQL Collector exports statistics only; it does not collect SQL text, query samples,
database contents, or application credentials. The inspector shows its metrics when the
PostgreSQL node or a calling service is selected.
