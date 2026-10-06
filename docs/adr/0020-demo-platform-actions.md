# ADR-0020: Container restart, scaling, feature flags and rollback as typed demo actions, run through a demo orchestrator

## Context
ADR-0012 shipped three actions: fault control and a restart of the demo database. The changes
operators actually make are a container restart, scaling replicas, switching a feature flag and
rolling back a release. Apart from Postgres, the demo services are plain Node processes under
`just demo`, not containers. Their "containers" on the System Map come from the demo infra agent.
The verification in ADR-0012 only counted a service's requests and errors. That cannot show that
a restarted instance serves traffic again, that the requested number of replicas serves traffic,
or that a rollback really runs the earlier release.

## Decision
- **Four new catalog entries, all demo-only, medium risk, with fixed targets:**
  - `demo.container.restart` on `gateway`, `orders`, `catalog-1..3`.
  - `demo.replicas.scale` on `catalog`, with `replicas` 1–3.
  - `demo.feature_flag.set` on `catalog-cache` (gateway) or `checkout-v2` (orders), with
    `enabled` 0/1.
  - `demo.release.rollback` on `orders`.
  Each one is a typed variant of `Action`. Targets and versions are enums or allowlists. Nothing
  comes from telemetry.
- **Switch parameters** are `boolean` in the catalog: an integer 0..1 on the wire (the API params
  stay numbers), rendered as a checkbox.
- **Demo orchestrator** (`scripts/demo-control.mjs`, 127.0.0.1:7090) plays the container platform
  inside `just demo`.
  - It restarts processes; a catalog replica is drained from the gateway first.
  - It starts or stops catalog replicas and pushes the replica list to the gateway
    (`/__demo/upstreams`).
  - A release is the service restarted with another `KARBIRD_SERVICE_VERSION`. Rollback returns to
    the previous version in the history; with no history it answers 409.
  - It accepts only the instance names, replica sets and versions it is configured with, and
    serialises operations.
  - `supervise()` lets it stop and start single processes without taking the demo down. An
    unexpected exit still stops everything.
- **Feature flags** live in the services: `/__demo/flags`, loopback only, only with
  `KARBIRD_DEMO_FAULTS=1`. Only declared flags exist and all start off.
  - `checkout-v2` makes every order write fail on purpose, so verification visibly catches a bad
    rollout.
  - Release `0.2.0` of orders fails 40% of order reads, so a rollback has something to fix.
  - `just demo-release orders 0.2.0` deploys it.
- **Every step reads its effect back** and fails if the effect differs from the request:
  - restart: a new PID, running;
  - scale: running and routed replica counts;
  - flags: flag state;
  - rollback: an allowlisted target version, different from the old one, reported as current.
- **Verification by metrics is per check** (`Verification` + `Observed`, one ClickHouse query over
  server spans of the service in the 15 s after the step):
  - restart: requests and errors of the restarted instance only (`service.instance.id`);
  - scale: exactly `replicas` distinct instances serve requests;
  - rollback: requests of the target release only (`service.version`), and zero requests from
    any other release. The release comes from the adapter result and must be allowlisted, or the
    plan fails verification without a query;
  - all four: at least one request and an error ratio of at most 5%.

## Consequences
- The executor stays the only process that changes the demo, and it reaches the orchestrator only
  over loopback. Web, API and ingest have no path to it.
- Flags, replica counts and releases are demo state in memory. Restarting `just demo` resets
  them (flags off, three replicas, orders 0.1.0).
- A flag rollout that breaks its service ends as `verification_failed` after the step has run.
  The flag stays on until someone switches it off with another plan.
- Real adapters (Kubernetes rollout restart and scale, a flag service, deploy tooling) can reuse
  the same typed actions and verification. The targets and adapters still need their own
  allowlists. They are not implemented.

## Alternatives rejected
- Running the demo services in Docker so restarts use `docker restart`: it would make `just demo`
  depend on building images, and scaling would need Compose replicas behind a proxy. That gains
  nothing over a supervised process.
- Generic "set environment variable" or "run version X" actions: they are unbounded and
  incompatible with ADR-0012's allowlist.
- Verifying a rollback by the service's error ratio alone: the old release could still be serving.
