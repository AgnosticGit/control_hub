# Karbird Control: one Docker container per installation

## Intent and scope

A project owner can connect a Docker host to one Karbird environment and explicitly allow one container. Authorized users can start, stop, or restart that container from the existing Actions page and the container's System Map card. Karbird Collector remains a separate, read-only installation. This first version supports Linux Docker Engine and one configured container per Control installation. It does not offer shell commands, image changes, container creation/deletion, or arbitrary Docker hosts.

## Existing architecture

ADR-0012 keeps action policy, approval, audit, and the plan state machine in the API/storage/executor path. ADR-0020 implements demo process restarts through a loopback orchestrator; those map nodes are not generally Docker containers. API and web have no Docker access. The new capability extends the typed action catalog and the executor; it does not reuse read-only API tokens or ingest tokens as control credentials.

## User experience and installation

An owner opens the environment's Control setup screen, chooses a display name and a container from the host, and requests an enrollment code. The screen shows a version-pinned Compose example for `karbird-control`, with the public HTTPS Karbird origin, the enrollment code supplied through a local secret file, a persistent state volume, and local Docker access. The code is shown once, expires after 15 minutes, and can be revoked. It contains no long-lived secret in a command line or published Compose file.

Control starts, connects outbound to Karbird over HTTPS, and exchanges the code for its own scoped credential. It then reports the Docker container ID, name, labels, state, and health. The owner confirms the exact container ID and enables only the desired three verbs. Before confirmation, no command can be dispatched. The screen shows connected/disconnected state, last contact, allowed target, and credential revocation. A disconnected Control does not hide telemetry or prevent Collector from running; action submission explains that execution will wait only within the normal plan deadline.

The installer must state that access to the Docker socket is effectively host-level privilege. An application allowlist limits what Karbird Control will intentionally operate on, but it is not a security boundary against compromise of that process. The image must be pinned to a version or digest, run with a read-only filesystem and no extra Linux capabilities where possible, and retain only its state directory and Docker socket. A narrower Docker authorization proxy is a future option, not a claim of this release.

## Identity and target binding

PostgreSQL stores a `control_installations` row scoped by authenticated tenant/project/environment, with an installation UUID, hashed enrollment code and expiry, hashed long-lived Control credential, revoked/confirmed flags, allowed Docker container ID, displayed name, last contact, and reported state. Secret material is never returned after enrollment or stored in plaintext. Enrollment and confirmation are audited.

The local configuration names exactly one Docker container ID, not a telemetry attribute. On first enrollment Control inspects that ID and sends its identity for owner confirmation. A renamed container retains the same ID. A deleted/recreated container requires a fresh confirmation, even if it reuses the name. The server checks the installation's scope and confirmed target before creating a plan; the executor checks them again before dispatch; Control checks the ID against its local configuration immediately before the Docker call. No target is derived from received telemetry.

All Control endpoints use a dedicated bearer credential whose hash is looked up only within the installation table. Tokens are scoped to one installation and environment, rotated on explicit request, and revocable. The enrollment endpoint accepts only a valid, unexpired, unused code and atomically consumes it. Authentication failures return generic errors and never include secrets in logs or audit payloads.

## Command path

The typed catalog adds `container.start`, `container.stop`, and `container.restart` for confirmed installations. Because current `Action::target()` and catalog targets are static demo strings, the action model and catalog response must support dynamic installation IDs. A proposal names the installation UUID; the server resolves its bound container ID from storage, never from client parameters or telemetry. The existing user permissions, CSRF handling, policy evaluation, one-open-plan-per-target rule (keyed by installation UUID across all three verbs), approval, expiration, and audit states apply. Stop is described as service-impacting; the UI requires a reason and shows the exact target and current state.

`karbird-executor` claims approved plans as today and inserts one command row for the plan in a transaction. Control polls outbound over HTTPS. The API atomically leases a command only to the matching installation. The command carries a unique ID, plan ID, verb, bound container ID, and expiry. Control records the command ID in a durable local journal before execution, applies it at most once, then submits the observed result. A retry of the same command returns the saved result without repeating a restart. Expired, revoked, mismatched, or already completed commands cannot execute.

The executor waits for the result while renewing its plan lease. Network loss or an executor crash must not silently reissue the Docker operation. If the outcome is unknown, the plan ends `failed` with `outcome_unknown` and requires human inspection. The command/result table and the Control journal preserve enough information for reconciliation and audit. The API never executes Docker calls.

## Local execution and verification

Control uses Docker Engine's local socket with a fixed method/path for each typed verb; it never invokes a shell. It inspects the exact container ID before and after the call. Start succeeds only when the container is running and, if a healthcheck exists, healthy by a bounded deadline. Stop succeeds only when it is stopped. Restart succeeds only when a new start time is observed and the container is running/healthy. A Docker error, wrong ID, unexpected state, or deadline expiration produces a failed result with a bounded, redacted diagnostic. No automatic rollback is implied.

Server-side verification records the attested before/after state, Control identity, command ID, and action timestamps. For start/restart, service telemetry may be shown as supporting evidence when available; it is not mandatory because Collector can be absent. Stop never depends on post-stop telemetry. The UI distinguishes operation failure from verification failure and from an unknown outcome.

## API, UI, deployment, and tests

New owner-only, CSRF-protected scoped routes create/list/revoke installations, issue enrollment codes, and confirm the observed container ID. Dedicated Control routes exchange the code, heartbeat/report status, poll commands, and submit results. API schemas are generated with `just codegen`. The existing Actions UI and System Map controls display only catalog entries bound to a confirmed installation; a setup page handles enrollment and status. Public pages remain free of browser-only engine imports.

The `karbird-control` package is a standalone Rust binary and versioned container image. Deploy configuration does not put its credential in the Karbird server image. Documentation includes Linux Docker/Compose installation, secret handling, rotation/revocation, upgrade, and uninstall. Collector setup is unchanged.

Tests cover typed validation and denied targets; owner/role/tenant/CSRF boundaries; expired or reused enrollment; credential revocation; command lease and duplicate result behavior; Docker ID mismatch; start/stop/restart success, failure, health timeout, and duplicate command journal; offline and unknown-outcome handling; API-to-Control integration with a disposable Docker container; and Playwright flows for setup, approval, status, and all three verbs. Run affected tests during development, then `just check`, `just test`, `just test-integration`, and `just test-e2e` with the dev/demo stack.

## Deliberate exclusions

No general remote Docker API exposure, no arbitrary command execution, no Kubernetes adapter, no bulk hosts/containers, no guarantee that Docker socket access is sandboxed by the one-container allowlist, and no automatic retry of an operation with an unknown result.
