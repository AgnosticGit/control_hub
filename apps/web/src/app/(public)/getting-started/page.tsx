import type { Metadata } from "next";
import Link from "next/link";
import { curlCheck, SDK_MIN_NODE, SDK_PACKAGE, sdkEnv, sdkInstall, sdkStart } from "@/components/integration/snippets";
import { Prose } from "../prose";

export const metadata: Metadata = {
  title: "Getting started",
  description: "Connect your first Node.js service to karbird with the karbird SDK and see it live on the service map.",
};

const example = { endpoint: "https://<your-endpoint>/otlp", token: null, serviceName: "checkout" };
const link = "font-semibold underline decoration-border-strong hover:decoration-text";

export default function GettingStartedPage() {
  return (
    <Prose title="Getting started" lead="Connect your first service in a few minutes. Everything goes through our API; there is nothing to install on our side.">
      <h2>1. Create an account</h2>
      <p>
        <Link href="/app/register" className={link}>
          Create an account
        </Link>
        . You get your own project with an environment straight away, and a short onboarding that walks through the next steps.
      </p>

      <h2>2. Create an ingestion token</h2>
      <p>
        In the workspace, open <strong>Integration</strong> and create a token. It is bound to one environment and shown once; we
        keep only its hash. The same page shows your endpoint and ready-to-copy settings with the token filled in.
      </p>

      <h2>3. Install the karbird SDK</h2>
      <p>
        <code>{SDK_PACKAGE}</code> works with Node.js {SDK_MIN_NODE} or newer, for CommonJS and ESM services:
      </p>
      <pre>{sdkInstall()}</pre>

      <h2>4. Start your service with the SDK</h2>
      <p>Set the endpoint and token from your Integration page:</p>
      <pre>{sdkEnv(example)}</pre>
      <p>Then start the service with the SDK loaded first. No code changes are needed:</p>
      <pre>{sdkStart()}</pre>
      <p>
        Incoming and outgoing HTTP requests and PostgreSQL queries are traced automatically. Headers, bodies and full SQL text are
        never recorded.
      </p>

      <h2>5. Check the connection</h2>
      <p>An empty export is a valid request, so this tells you the endpoint and token work before any service is changed:</p>
      <pre>{curlCheck(example)}</pre>
      <p>The Integration page shows when the first data arrives.</p>

      <h2>6. Open the map</h2>
      <p>
        Services appear on the map as their spans arrive, with the calls between them. Drag the timeline back to see any earlier
        moment, or click a service for its traces, logs and events.
      </p>

      <h2>Next</h2>
      <ul>
        <li>
          Invite your team and set their roles under <strong>Members &amp; roles</strong>.
        </li>
        <li>
          Give your AI assistant read-only context with the <code>karbird</code> CLI and MCP server, see{" "}
          <Link href="/integrations" className={link}>
            Integrations
          </Link>
          .
        </li>
        <li>
          See what is available today and what is coming on the{" "}
          <Link href="/features" className={link}>
            Features
          </Link>{" "}
          page.
        </li>
      </ul>
    </Prose>
  );
}
