# Change Replay

Change Replay lists `deployment.finished` events in the selected project and environment, then compares service request counts, errors, error rates and p95 latency in equal windows before and after the event. The default maximum is 15 minutes per side. A recent release uses the elapsed post-release time on both sides and is marked provisional. The page shows processor lag, failed processing and truncation when reported by the API. Differences are observations, not proof of causation.

## Record a deployment

After a deployment has finished, send one event to the ingest endpoint with the ingest token for that environment:

```json
{
  "events": [
    {
      "event_id": "<new UUID>",
      "event_type": "deployment.finished",
      "service": { "name": "orders" },
      "event_time": "<Unix nanoseconds as a decimal string>",
      "attributes": {
        "service.version": "2.0.0",
        "deployment.from_version": "1.9.0"
      }
    }
  ]
}
```

Use `POST /api/v1/events/batch` on the ingest service and `Authorization: Bearer <environment ingest token>`. Send the event after the deployment tool confirms completion; the timestamp is the change boundary for the comparison. Do not include credentials or private payloads in attributes. Ingest redaction still applies.

For the local demo, `just demo-release orders 0.2.0` emits this event after a successful deployment. The demo rollback action emits another event with `deployment.kind=rollback` after it completes. The event has broker-acknowledged durability before it is shown in Change Replay.

The page currently reads the last seven days and at most 1,000 matching events. It shows a truncation warning when more exist. The existing Actions page remains the only way to propose or run a controlled change; Change Replay links there for review. Production action adapters are not implemented yet.
