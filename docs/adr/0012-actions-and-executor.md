# ADR-0012: Controlled actions — plans, policy, four-eyes approval, separate executor

## Decision
- **Allowlist, typed.** Actions exist only in the `controlhub-actions` catalog, as a Rust `Action`
  enum with bounded numeric parameters and fixed targets. There is no free-form command, script,
  URL or container name anywhere in a plan. Telemetry never produces a plan.
- **Plans.** An operator proposes a plan with a reason. Policy is evaluated at proposal, at approval
  and again by the executor right before it runs:
  - demo-only actions are allowed only in `is_demo` environments;
  - at most one open plan per target;
  - at most 30 plans per environment per hour.
  Denials are audited too.
- **Four-eyes approval.** Every plan needs approval by an admin other than the proposer. This is
  enforced in the API and by a `CHECK (decided_by <> created_by)` constraint on approved plans.
  Rejecting a plan requires a comment. The proposer or an admin may cancel a plan before it runs.
- **Windows.** A plan must be decided within 30 min and started within 10 min of approval. After
  that the executor marks it expired and never runs it.
- **State machine.** proposed → approved | rejected | cancelled | expired; approved → running |
  cancelled | expired; running → succeeded | failed | verification_failed. Transitions are guarded
  in SQL (`WHERE state = ANY(from)`), and every transition commits together with its audit row.
  If the audit write fails, the state change fails.
- **Separate executor process** (`controlhub-executor`):
  - It has its own PostgreSQL role (`SELECT, UPDATE` on `action_plans`, `INSERT` on `audit_log`,
    read-only elsewhere) and a SELECT-only ClickHouse user.
  - It is the only process allowed to touch the observed system or Docker. Web, API and ingest
    never get the Docker socket.
  - It claims a plan with `FOR UPDATE SKIP LOCKED` and a 60 s lease, renewed while it works.
  - If the lease lapses (executor crashed), the plan is marked `failed` with "outcome unknown" and
    is **never retried** automatically. A human decides.
- **Demo adapter.**
  - Fault control uses the demo services' loopback-only `/__demo/faults` endpoint and reads the
    state back.
  - The database restart runs `docker inspect`/`docker restart` with fixed arguments (no shell) on a
    container whose compose labels must be `controlhub/demo-postgres`. The container name must start
    with an alphanumeric character, so it can never be parsed as a flag.
- **Verification.** After the step, the executor waits for the observation window (15 s) plus an
  ingestion settle time. It then judges the outcome from ClickHouse `service_summaries` for the
  plan's scope (request count, error ratio). A step that ran but fails verification ends as
  `verification_failed`, not `succeeded`.

## Consequences
- A lost executor leaves a visible `failed` plan rather than a silent retry, so non-idempotent
  actions (restarts) never run twice by accident.
- Verification is statistical, over a short window. It proves the service is receiving traffic
  with a bounded error ratio, not that the action caused that state.
- Only the demo adapter exists. Real adapters (Kubernetes, feature flags, …) need their own
  allowlisted actions, targets and verification. They are not implemented.

## Alternatives rejected
- Running actions inside the API: it would give the internet-facing process side-effect
  privileges (and possibly the Docker socket).
- Automatic retry on executor loss: unsafe for non-idempotent actions.
- Free-form runbooks/commands: incompatible with "never execute telemetry-derived input" and
  impossible to verify or bound.
