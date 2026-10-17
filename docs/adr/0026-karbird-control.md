# ADR-0026: Outbound control of one customer Docker container

## Context

ADR-0012 requires typed, audited and approved actions through a separate executor. ADR-0020 controls demo processes and the local demo database; it does not control Docker on a customer's host. A customer with one container needs optional start, stop and restart controls without installing write privileges in Collector or exposing Docker's remote API.

## Decision

- `karbird-control` is a separate Linux service installed beside Docker, configured with exactly one full container ID. It connects outbound to the Karbird HTTPS API and uses the local Docker Unix socket. Collector remains independent and read-only.
- A project manager creates a scoped installation and gets a one-time, 15-minute enrollment code. The code is consumed atomically and replaced with a distinct hashed Control credential. The reported Docker ID requires explicit owner confirmation; a recreated container must be confirmed as a new installation.
- The existing `actions.propose` and `actions.approve` permissions, policy, plan state machine and audit apply to `container.start`, `container.stop` and `container.restart`. These actions use an installation UUID as their target. API and web never access Docker.
- The executor inserts one command per approved plan. Control polls for it, persists its command ID before executing, calls only a fixed Docker method/path, and reads the exact container state back. Duplicate delivery returns the journaled result without repeating the operation. If execution becomes ambiguous, the plan reports an unknown outcome and no automatic retry occurs.
- Start/restart verify running and healthy when a Docker healthcheck exists; restart also requires a changed start time. Stop verifies Docker's stopped state and never waits for telemetry from the stopped service.
- All credentials are stored as hashes server-side and as a local secret file on the customer host. Revocation disables polling. The customer must preserve the local state volume across upgrades.

## Security boundary

Docker socket access is effectively host-level privilege. An application-level one-container allowlist reduces mistakes and limits the commands Karbird Control intentionally sends; it does not sandbox a compromised Control process. The installation guide states this explicitly. The service has no inbound port, no arbitrary shell/command endpoint, and no access to user-supplied URLs or telemetry-derived targets.

## Consequences

This first version supports Linux Docker Engine and one container per Control installation. It does not manage Kubernetes, images, or container creation. Offline installations can leave plans with unknown outcomes; a human must inspect Docker before submitting another plan. A published version-pinned image and end-to-end verification on a Linux Docker host are required before a production rollout.
