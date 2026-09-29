import type { Metadata } from "next";
import { Prose } from "../prose";

export const metadata: Metadata = {
  title: "Getting started",
  description: "Run ControlHub and the demo system locally with Docker, Rust and Node.js.",
};

export default function GettingStartedPage() {
  return (
    <Prose title="Getting started" lead="Run the full stack and a demo system on your machine.">
      <h2>Prerequisites</h2>
      <ul>
        <li>Docker with Compose v2, Rust (pinned by rust-toolchain.toml), Node.js 22+, pnpm 10, just.</li>
        <li>About 3 GB of free RAM for PostgreSQL, ClickHouse and Kafka.</li>
      </ul>
      <h2>Steps</h2>
      <pre>{`just doctor      # check tools
just bootstrap   # generate local secrets, start infra, migrate, seed
just dev         # ingest + processor + API + web on http://localhost:3000
just demo        # demo services + load generator (second terminal)`}</pre>
      <p>
        Login credentials are generated per machine and written to <code>.controlhub/local-credentials.json</code>{" "}
        (gitignored). There is no shared default password.
      </p>
      <h2>Your own account and services</h2>
      <p>
        In dev mode anyone can create an account at <code>/app/register</code> (sign-up is closed by default outside dev,
        see <code>CONTROLHUB_SIGNUP</code>). A new account gets its own project; its <strong>Integration</strong> page
        creates an ingestion token and shows the OpenTelemetry settings for your services.
      </p>
      <pre>{`just dev ngrok   # like just dev, plus the dev stand at CONTROLHUB_PUBLIC_URL through ngrok`}</pre>
      <p>
        The tunnel exposes only the web app: pages, <code>/api/v1</code> and the OTLP endpoint <code>/otlp</code>. It is a
        development stand, not a production deployment.
      </p>
      <h2>Inject a failure</h2>
      <pre>{`just demo-fault orders --latency 800 --error-rate 0.3
just demo-fault orders --clear`}</pre>
    </Prose>
  );
}
