# E2E Regression Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve the browser-test failures observed in the full Playwright run while preserving the current product behavior and the demo limit of 10 external requests per minute.

**Architecture:** Trace each failure family to its UI behavior, fixture lifecycle, or test expectation. Fix product behavior when it violates the current route or interaction contract; update tests only where the rendered UI contract intentionally changed. Keep demo throughput bounded and make time-dependent tests use deterministic telemetry or rate-appropriate assertions.

**Tech Stack:** Next.js, React, TypeScript, Rust API, Node demo system, Playwright, Vitest.

**Spec:** User request to fix all failures from the previous full `just test-e2e` run.

## Global Constraints

- Keep external demo requests at 10 per minute.
- Preserve tenant scoping, no unsafe telemetry rendering, and no Git state changes.
- Keep Playwright selectors based on roles, labels, and visible text.
- Do not reset the local database or alter non-fixture scopes. The test lifecycle may clear telemetry only from its reserved `isolation-test` scope.

## Review Focus

- Single-environment users should enter their only workspace; multiple-environment users should get a picker.
- New accounts should complete setup before tests assume they have a project.
- Map overlays and popovers must not intercept pointer events outside their visible bounds.
- Time-sensitive telemetry tests must remain deterministic with the lower demo request rate.
- Timeline actions and accessible labels must match the current UI.

---

### Task 1: Workspace entry and account navigation

**Files:** `apps/web/src/app/app/page.tsx`, `apps/web/src/components/shell/AppShell.tsx`, relevant `tests/e2e/specs/*.spec.ts`.

- [ ] Redirect users with exactly one accessible environment to that workspace.
- [ ] Keep the environment picker for users with multiple environments and name it clearly.
- [ ] Restore visible demo context in the workspace scope control where the UI contract requires it.
- [ ] Align account and auth tests with signup's onboarding-first flow.
- [ ] Run the relevant navigation, auth, account, onboarding, and accessibility Playwright tests.

### Task 2: Map and infrastructure interactions

**Files:** `apps/web/src/components/workspace/MapView.tsx`, `apps/web/src/components/workspace/InfraOverlay.tsx`, `apps/web/src/components/workspace/Workspace.tsx`, relevant `tests/e2e/specs/*.spec.ts`.

- [ ] Reproduce pointer interception from map overlays and confine hit targets to visible controls.
- [ ] Preserve map keyboard, fullscreen, table, and infrastructure interactions.
- [ ] Update selectors only for controls intentionally replaced by scope selectors or removed map title chrome.
- [ ] Run relevant map, infrastructure, and accessibility Playwright tests.

### Task 3: Demo telemetry and timeline tests

**Files:** `examples/demo-system/src/loadgen.mjs`, `scripts/demo-incidents.mjs`, relevant timeline and signal tests.

- [ ] Keep the external load generator capped at 10 requests per minute.
- [ ] Ensure tests that need dense traces, logs, metrics, or events inject deterministic fixtures instead of relying on recent demo traffic.
- [ ] Fix any timeline loading or cursor behavior exposed by deterministic tests.
- [ ] Run the relevant signal, timeline, external-system, and topology Playwright tests.

### Task 4: Full verification

**Files:** All files changed in Tasks 1–3.

- [ ] Run focused web tests and typecheck.
- [ ] Run the complete Playwright suite against `just dev` and `just demo`.
- [ ] Review remaining failures and rerun only after fixing their root causes.
- [ ] Run `just check` and `just test`; report any environment-dependent suite that cannot run.
