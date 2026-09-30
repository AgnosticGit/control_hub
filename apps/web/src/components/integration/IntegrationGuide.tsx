"use client";

import Link from "next/link";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useState, useSyncExternalStore, type ReactNode } from "react";
import { Icon } from "@/components/ui/Icon";
import { api, ApiRequestError, scopedPath, unwrap } from "@/lib/api";
import { collectorConfig, curlCheck, nodeSteps, otelEnv, otlpEndpoint, safeServiceName } from "./snippets";

const TOKENS = "/api/v1/projects/{project_id}/environments/{environment_id}/tokens" as const;
const SERVICES = "/api/v1/projects/{project_id}/environments/{environment_id}/services" as const;

const noSubscribe = () => () => {};

function Snippet({ label, text }: { label: string; text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="relative">
      <pre aria-label={label} className="overflow-x-auto rounded-lg border border-border bg-bg/80 p-3 pr-20 font-mono text-xs leading-relaxed">
        {text}
      </pre>
      <button
        type="button"
        className="btn btn-sm absolute top-2 right-2"
        aria-label={`Copy ${label}`}
        onClick={() => {
          void navigator.clipboard.writeText(text).then(() => setCopied(true));
        }}
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function Step({ n, title, children }: { n: number; title: string; children: ReactNode }) {
  return (
    <section aria-labelledby={`step-${n}`} className="card space-y-3 p-5 text-sm">
      <h2 id={`step-${n}`} className="flex items-center gap-2 text-base font-semibold">
        <span className="flex h-6 w-6 items-center justify-center rounded-full bg-accent/20 text-xs text-accent">{n}</span>
        {title}
      </h2>
      {children}
    </section>
  );
}

/** Services that sent telemetry to the environment in the last 15 minutes, polled every 5 seconds. */
export function useSeenServices(projectId: string, environmentId: string) {
  return useQuery({
    queryKey: ["integration-services", projectId, environmentId],
    enabled: Boolean(projectId && environmentId),
    refetchInterval: 5000,
    queryFn: async () => unwrap(await api.GET(SERVICES, { params: { path: scopedPath(projectId, environmentId) } })),
  });
}

/** Ingestion token, configuration snippets and a live "receiving data" check for one environment. */
export function IntegrationGuide({
  projectId,
  environmentId,
  canCreateToken,
}: {
  projectId: string;
  environmentId: string;
  canCreateToken: boolean;
}) {
  const path = scopedPath(projectId, environmentId);
  const origin = useSyncExternalStore(noSubscribe, () => window.location.origin, () => "");
  const [service, setService] = useState("my-service");
  const [secret, setSecret] = useState<string | null>(null);
  const base = `/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}`;

  const create = useMutation({
    mutationFn: async () =>
      unwrap(
        await api.POST(TOKENS, {
          params: { path },
          body: { kind: "ingest", name: `integration ${safeServiceName(service)}`.slice(0, 64), expires_in_days: null },
        }),
      ),
    onSuccess: (r) => setSecret(r.secret),
  });

  const services = useSeenServices(projectId, environmentId);

  const input = { endpoint: origin ? otlpEndpoint(origin) : "", token: secret, serviceName: service };
  const seen = services.data?.services ?? [];

  return (
    <>
      <div className="text-sm text-muted">
        <p>
          ControlHub receives standard OpenTelemetry traces and logs over OTLP/HTTP (protobuf). Any language with an OpenTelemetry
          SDK works; no ControlHub-specific library is required. Telemetry for this environment goes to:
        </p>
        <p className="mt-2">
          <code aria-label="OTLP endpoint" className="rounded-md border border-border bg-bg/80 px-2 py-1 font-mono text-xs text-text">
            {input.endpoint}
          </code>
        </p>
      </div>

      <Step n={1} title="Create an ingestion token">
        <p className="text-muted">
          The token decides which environment the data belongs to. It can only send telemetry, never read it. It is shown once and
          stored only as a hash; revoke or rotate it on the{" "}
          <Link href={`${base}/tokens`} className="text-accent hover:underline">
            Tokens
          </Link>{" "}
          page.
        </p>
        {canCreateToken ? (
          <form
            className="flex flex-wrap items-end gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              create.mutate();
            }}
          >
            <label className="flex flex-col gap-1.5">
              <span className="label">Service name</span>
              <input value={service} maxLength={64} onChange={(e) => setService(e.target.value)} className="input h-9 w-56" />
            </label>
            <button type="submit" disabled={create.isPending} className="btn btn-primary h-9 px-4">
              <Icon name="key" className="h-4 w-4" />
              Create ingestion token
            </button>
          </form>
        ) : (
          <p className="rounded-lg border border-border bg-surface-2 px-3 py-2">
            Creating tokens needs the “Create, rotate and revoke tokens” permission. Ask a project admin for an ingestion token.
          </p>
        )}
        {create.error ? (
          <p role="alert" className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error">
            {create.error instanceof ApiRequestError ? create.error.message : "Could not create the token."}
          </p>
        ) : null}
        {secret ? (
          <div role="status" className="rounded-xl border border-warn/50 bg-warn/10 p-3">
            <p className="font-medium">Copy the token now; it will not be shown again. The snippets below already contain it.</p>
            <code aria-label="New ingestion token" className="mt-2 block font-mono text-xs break-all">
              {secret}
            </code>
          </div>
        ) : null}
      </Step>

      <Step n={2} title="Configure your service">
        <h3 className="font-medium">Any language (OpenTelemetry SDK)</h3>
        <p className="text-muted">
          Set these environment variables for a service instrumented with the official OpenTelemetry SDK (Java, Python, Go, .NET,
          Node.js, Ruby, PHP, …). Metrics are not stored yet, so their exporter is turned off.
        </p>
        <Snippet label="OpenTelemetry environment variables" text={otelEnv(input)} />

        <h3 className="pt-2 font-medium">Node.js without code changes</h3>
        <Snippet label="Node.js setup" text={nodeSteps(input)} />

        <h3 className="pt-2 font-medium">Through an OpenTelemetry Collector</h3>
        <p className="text-muted">If your services already send to a Collector, add ControlHub as one more exporter.</p>
        <Snippet label="Collector configuration" text={collectorConfig(input)} />
      </Step>

      <Step n={3} title="Check the connection">
        <p className="text-muted">
          A <code className="font-mono">200</code> response means the token and endpoint are right;{" "}
          <code className="font-mono">401</code> means the token is wrong, revoked or expired.
        </p>
        <Snippet label="Connection check" text={curlCheck(input)} />
        <div role="status" aria-label="Telemetry status" className="flex items-center gap-2 rounded-lg border border-border bg-surface-2 px-3 py-2">
          {seen.length > 0 ? (
            <>
              <span className="dot bg-ok" aria-hidden />
              <span>
                Receiving data from {seen.length} service{seen.length === 1 ? "" : "s"}: {seen.map((s) => s.name).join(", ")}.{" "}
                <Link href={base} className="text-accent hover:underline">
                  Open the overview
                </Link>
              </span>
            </>
          ) : (
            <>
              <span className="dot bg-muted" aria-hidden />
              <span>Waiting for the first telemetry (checked every 5 seconds, last 15 minutes).</span>
            </>
          )}
        </div>
      </Step>

      <section aria-labelledby="privacy" className="card space-y-2 p-5 text-sm">
        <h2 id="privacy" className="text-base font-semibold">
          What is stored
        </h2>
        <ul className="list-disc space-y-1 pl-5 text-muted">
          <li>Spans (service, operation, timing, status, redacted attributes) and log records.</li>
          <li>
            Authorization and cookie headers, passwords, tokens and similar secrets are removed on arrival, even if your SDK sent
            them. Do not put request bodies or personal data into span attributes or log messages.
          </li>
          <li>Only protobuf encoding is accepted (JSON gets 415). Metrics are refused with 501 and not stored.</li>
        </ul>
      </section>
    </>
  );
}
