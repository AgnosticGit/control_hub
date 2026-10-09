"use client";

import Link from "next/link";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useState, useSyncExternalStore, type ReactNode } from "react";
import { Icon } from "@/components/ui/Icon";
import { ErrorNotice } from "@/components/ui/ErrorNotice";
import { api, ApiRequestError, scopedPath, unwrap } from "@/lib/api";
import {
  curlCheck,
  otlpEndpoint,
  phpSdkBootstrap,
  phpSdkEnv,
  phpSdkInstall,
  safeServiceName,
  SDK_MIN_NODE,
  SDK_PACKAGE,
  sdkCode,
  sdkEnv,
  sdkInstall,
  sdkStart,
} from "./snippets";

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
  const [language, setLanguage] = useState<"node" | "php">("node");
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
  const isPhp = language === "php";
  const seen = services.data?.services ?? [];

  return (
    <>
      <div className="text-sm text-muted">
        <div role="group" aria-label="Choose service language" className="mb-4 inline-flex rounded-lg border border-border bg-surface-2 p-1">
          {([ ["node", "Node.js"], ["php", "PHP"] ] as const).map(([id, label]) => (
            <button
              key={id}
              type="button"
              aria-pressed={language === id}
              onClick={() => setLanguage(id)}
              className={`rounded-md px-3 py-1.5 text-sm ${language === id ? "bg-accent text-white" : "text-muted hover:text-text"}`}
            >
              {label}
            </button>
          ))}
        </div>
        <p>
          {isPhp ? (
            <>Connect PHP services with <code className="font-mono text-text">karbird/sdk-php</code>. The host agent remains language-independent and collects machine and container metrics.</>
          ) : (
            <>Connect Node.js services with the karbird SDK <code className="font-mono text-text">{SDK_PACKAGE}</code> (Node.js {SDK_MIN_NODE} or
            newer). It traces incoming and outgoing HTTP requests and PostgreSQL queries, and sends logs and events linked to them.</>
          )}{" "}
          Telemetry for this environment goes to:
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
        {create.error ? <ErrorNotice>{create.error instanceof ApiRequestError ? create.error.message : "Could not create the token."}</ErrorNotice> : null}
        {secret ? (
          <div role="status" className="rounded-xl border border-warn/50 bg-warn/10 p-3">
            <p className="font-medium">Copy the token now; it will not be shown again. The snippets below already contain it.</p>
            <code aria-label="New ingestion token" className="mt-2 block font-mono text-xs break-all">
              {secret}
            </code>
          </div>
        ) : null}
      </Step>

      <Step n={2} title="Install the SDK">
        {isPhp ? (
          <>
            <p className="text-muted">From your PHP service directory, while the Karbird monorepo is checked out alongside it:</p>
            <Snippet label="Install PHP SDK" text={phpSdkInstall()} />
            <p className="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-muted">
              The PHP package is currently installed from the local monorepo path. Automatic framework and database spans also need
              the <code className="font-mono">ext-opentelemetry</code> PHP extension and matching OpenTelemetry instrumentation package
              for your framework/client.
            </p>
          </>
        ) : (
          <>
            <p className="text-muted">In the directory of your service:</p>
            <Snippet label="Install command" text={sdkInstall()} />
          </>
        )}
      </Step>

      <Step n={3} title="Start your service with the SDK">
        <p className="text-muted">
          Put these variables into your <code className="font-mono">.env</code>, process manager or container environment:
        </p>
        <Snippet label="SDK environment variables" text={isPhp ? phpSdkEnv(input) : sdkEnv(input)} />
        {isPhp ? (
          <>
            <p className="text-muted">
              Make these variables available to PHP-FPM or the container. Load Karbird before Composer&apos;s autoloader so OpenTelemetry
              starts with the correct endpoint and token:
            </p>
            <Snippet label="PHP bootstrap" text={phpSdkBootstrap()} />
            <p className="text-xs text-muted">
              For Laravel, install its OpenTelemetry auto-instrumentation package as well. Other frameworks and database clients need
              their corresponding instrumentation package.
            </p>
          </>
        ) : (
          <>
            <p className="text-muted">
              Start the service with the SDK loaded first, so it sees every request from the start. Replace{" "}
              <code className="font-mono">server.js</code> with your entry file; no code changes are needed.
            </p>
            <Snippet label="Start command" text={sdkStart()} />
            <h3 className="pt-2 font-medium">Logs and events from your code (optional)</h3>
            <Snippet label="Logs and events from code" text={sdkCode()} />
          </>
        )}
      </Step>

      <Step n={4} title="Check the connection">
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
          <li>Spans (service, operation, timing, status, redacted attributes), log records and events.</li>
          <li>The SDK never records request or response headers, bodies or full SQL text.</li>
          <li>
            Authorization and cookie headers, passwords, tokens and similar secrets are also removed on arrival. Do not put personal
            data into log messages or event attributes.
          </li>
        </ul>
      </section>
    </>
  );
}
