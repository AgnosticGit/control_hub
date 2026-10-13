# ADR-0024: Service incident queue

Status: accepted

## Context

Operators need a durable, scoped queue for sustained request errors and latency, with a link back to the observed moment. Existing ClickHouse service summaries provide request counts, errors, and p95 latency from spans. PostgreSQL already holds project roles, audit records, and processor watermarks.

## Decision

The API evaluates configured per-service rules every 15 seconds against closed rolling five-minute windows, advancing one minute at a time. It waits until all processor partition offsets are caught up and their watermarks were refreshed after the window closed. Unresolved processing errors pause evaluation for that tenant. A rule is **unknown** when fewer than its minimum requests are present; unknown evidence breaks a streak but does not recover an open incident.

Two adjacent breaching windows open one incident; two adjacent healthy windows recover it. A skipped minute breaks a streak. On restart, evaluation catches up at most ten minutes of closed windows. PostgreSQL row locks, a partial unique index, and a per-window cursor make transitions idempotent. An advisory transaction lock prevents API replicas from running the same batch concurrently. Incidents retain their scoped service, threshold, evidence, acknowledgment, and lifecycle state. Rule changes close the prior open incident and reset evaluation.

Project members with `telemetry.view` may read the queue and rules. `incidents.manage` permits creating, changing, pausing, and acknowledging; built-in operator, admin, and owner roles receive it. Browser mutations use session, Origin, and CSRF protections. The queue links to the historical workspace using a decimal nanosecond timestamp and service ID.

## Consequences

Evaluation follows the processor's durable visibility rather than speculative current traffic. It can lag by the two-minute settling period plus scheduler and processing time. Rolling windows share data, so a sustained short burst can appear in two consecutive evaluations; the minimum request count and explicit threshold remain important. Rules cover request spans only, not logs or host metrics. This design adds no new broker consumer or external alerting channel.
