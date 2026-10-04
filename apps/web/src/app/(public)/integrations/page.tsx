import type { Metadata } from "next";
import Link from "next/link";
import { SDK_MIN_NODE, SDK_PACKAGE, sdkCode, sdkEnv, sdkInstall, sdkStart } from "@/components/integration/snippets";
import { Prose } from "../prose";

export const metadata: Metadata = {
  title: "Integrations",
  description: "Connect Node.js services to karbird with the karbird SDK.",
};

const example = { endpoint: "https://<your-endpoint>/otlp", token: null, serviceName: "orders" };
const link = "font-semibold underline decoration-border-strong hover:decoration-text";

export default function IntegrationsPage() {
  return (
    <Prose title="Integrations" lead="Services connect to karbird with one package: the karbird SDK.">
      <h2>Node.js services</h2>
      <p>
        <code>{SDK_PACKAGE}</code> runs on Node.js {SDK_MIN_NODE} or newer. It traces incoming and outgoing HTTP requests and
        PostgreSQL queries, and sends logs and request events linked to those traces. Install it:
      </p>
      <pre>{sdkInstall()}</pre>
      <p>Configure it with the endpoint and the ingestion token from your Integration page, and load it before your application code:</p>
      <pre>{`${sdkEnv(example)}\n\n${sdkStart()}`}</pre>
      <p>Headers, request and response bodies and full SQL text are never recorded.</p>

      <h2>Logs and events from your code</h2>
      <p>The running SDK instance gives you a logger and an events client. Both are linked to the request being handled:</p>
      <pre>{sdkCode()}</pre>
      <p>Events such as deployments and configuration changes appear on the timeline next to the traces.</p>

      <h2>Coding assistants</h2>
      <p>
        The <code>karbird</code> CLI and its read-only MCP server give your coding assistant the topology, errors and recent history
        of one environment. See{" "}
        <Link href="/getting-started" className={link}>
          Getting started
        </Link>{" "}
        for connecting your first service.
      </p>
    </Prose>
  );
}
