import Link from "next/link";
import { MapDemo } from "@/components/landing/MapDemo";
import { sdkEnv, sdkInstall, sdkStart } from "@/components/integration/snippets";

const outcomes = [
  {
    title: "See which service broke, and what it took down with it.",
    body: "A live map built from your traces. Failing and degraded services, and the calls between them, change colour the moment it happens.",
  },
  {
    title: "Rewind to the minute before it broke.",
    body: "Every moment is rebuilt from recorded telemetry. Pause, scrub and replay the whole system; deploys and actions sit on the same timeline.",
  },
  {
    title: "Follow one request through every service.",
    body: "Trace waterfall with the correlated logs and events, one click from the map. No copying IDs between tools.",
  },
];

const connect = [
  { name: "Create an account", note: "You get a project and an environment straight away." },
  { name: "Create an ingestion token", note: "On the Integration page, scoped to one environment." },
  { name: "Start your service with the karbird SDK", note: "The map fills in as the first requests arrive." },
];

const trust = [
  { title: "Nothing silently dropped", body: "We acknowledge telemetry only once it is safely in the pipeline. If we can't take it, your exporter gets an error, not a fake success." },
  { title: "Secrets redacted on arrival", body: "Authorization headers, cookies, passwords and tokens are stripped at ingestion, even if your SDK forgot to." },
  { title: "Your data stays yours", body: "Every query is scoped to your account, project and environment. Tokens are stored only as hashes." },
  { title: "No guessing on the map", body: "Each dependency says whether it was observed or inferred, and why. Gaps in data are shown as gaps." },
];

const sdkExample = [
  sdkInstall(),
  "",
  sdkEnv({ endpoint: "https://<your-endpoint>/otlp", token: null, serviceName: "checkout" }),
  "",
  sdkStart(),
].join("\n");

export default function HomePage() {
  return (
    <>
      <section className="mx-auto max-w-7xl px-6 pt-16 pb-20 lg:pt-20">
        <div className="grid items-end gap-x-16 gap-y-6 lg:grid-cols-[minmax(0,7fr)_minmax(0,5fr)]">
          <h1 className="text-[2.5rem] leading-[1.05] font-extrabold tracking-[-0.03em] text-balance sm:text-[3.25rem]">
            See what your system is doing{"\u00a0"}— and what it was doing five minutes ago.
          </h1>
          <div>
            <p className="max-w-xl text-[17px] leading-relaxed text-muted">
              karbird is a hosted workspace for your distributed system. Connect your services with the karbird SDK and get a
              live service map, a timeline you can rewind, and a safe way to act on what you see.
            </p>
            <div className="mt-7 flex flex-wrap items-center gap-3">
              <Link href="/app/register" className="btn btn-primary h-11 px-5 text-sm">
                Create account
              </Link>
              <Link href="/features" className="btn h-11 px-5 text-sm">
                How it works
              </Link>
            </div>
          </div>
        </div>
        <div className="mt-12">
          <MapDemo />
          <p className="mt-3 text-sm text-balance text-muted">
            Live demo on synthetic data: the same map and timeline you get in your workspace. Try dragging the timeline back.
          </p>
        </div>
      </section>

      <section aria-labelledby="outcomes" className="border-t border-border">
        <div className="mx-auto grid max-w-7xl gap-12 px-6 py-20 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
          <div>
            <h2 id="outcomes" className="text-3xl font-bold tracking-[-0.02em] text-balance">
              From “something is red” to the cause, without switching tools.
            </h2>
            <p className="mt-4 max-w-md leading-relaxed text-muted">
              Incidents are lost in the first minutes: finding the service, finding when it started, finding the request that shows
              why. karbird puts all three on one screen.
            </p>
          </div>
          <ol className="divide-y divide-border border-y border-border">
            {outcomes.map((o, i) => (
              <li key={o.title} className="grid gap-x-6 gap-y-2 py-7 sm:grid-cols-[3rem_minmax(0,1fr)]">
                <span className="font-mono text-sm text-muted tabular-nums" aria-hidden>
                  0{i + 1}
                </span>
                <div>
                  <h3 className="text-lg font-semibold text-balance">{o.title}</h3>
                  <p className="mt-2 max-w-xl leading-relaxed text-muted">{o.body}</p>
                </div>
              </li>
            ))}
          </ol>
        </div>
      </section>

      <section aria-labelledby="connect" className="border-t border-border bg-bg-2">
        <div className="mx-auto grid max-w-7xl items-start gap-12 px-6 py-20 lg:grid-cols-2">
          <div>
            <h2 id="connect" className="text-3xl font-bold tracking-[-0.02em] text-balance">
              One package, one start command. No agent, no code changes.
            </h2>
            <p className="mt-4 max-w-lg leading-relaxed text-muted">
              Install the karbird SDK for Node.js and load it when your service starts. HTTP requests, PostgreSQL queries,
              logs and events arrive linked to each other.
            </p>
            <ol className="mt-8 space-y-5">
              {connect.map((s, i) => (
                <li key={s.name} className="flex gap-4">
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-border-strong font-mono text-xs" aria-hidden>
                    {i + 1}
                  </span>
                  <span>
                    <span className="block font-semibold">{s.name}</span>
                    <span className="mt-0.5 block text-sm text-muted">{s.note}</span>
                  </span>
                </li>
              ))}
            </ol>
            <Link href="/getting-started" className="mt-8 inline-block text-sm font-semibold underline decoration-border-strong hover:decoration-text">
              Read the setup guide
            </Link>
          </div>
          <pre className="overflow-x-auto rounded-lg border border-border bg-bg p-5 font-mono text-[13px] leading-relaxed text-text">
            <code>
              <span className="text-muted"># karbird SDK for Node.js{"\n"}</span>
              {sdkExample}
            </code>
          </pre>
        </div>
      </section>

      <section aria-labelledby="act" className="border-t border-border">
        <div className="mx-auto grid max-w-7xl gap-16 px-6 py-20 lg:grid-cols-2">
          <div>
            <h2 id="act" className="text-3xl font-bold tracking-[-0.02em] text-balance">
              Change production with a plan and a second pair of eyes.
            </h2>
            <p className="mt-4 max-w-lg leading-relaxed text-muted">
              Actions are typed and bounded, never free-form commands. Each one is checked against policy, approved by a second
              person, run by a separate executor, verified from telemetry and written to the audit trail.
            </p>
            <ol className="mt-8 flex flex-wrap gap-2 text-sm" aria-label="Action flow">
              {["Plan", "Policy", "Approval", "Execution", "Verification", "Audit"].map((step) => (
                <li key={step} className="rounded-md border border-border bg-surface px-3 py-1.5">
                  {step}
                </li>
              ))}
            </ol>
            <p className="mt-4 text-sm text-muted">Available on the demo system today; adapters for real infrastructure are on the roadmap.</p>
          </div>
          <div>
            <h2 id="assistants" className="text-3xl font-bold tracking-[-0.02em] text-balance">
              Give your AI assistant the real picture, read-only.
            </h2>
            <p className="mt-4 max-w-lg leading-relaxed text-muted">
              The <code className="font-mono text-[0.9em] text-text">karbird</code> CLI and MCP server hand your coding assistant the
              topology, errors and recent history of one environment, with a read-only token. It can look; it cannot change anything.
            </p>
            <pre className="mt-6 overflow-x-auto rounded-lg border border-border bg-bg-2 p-5 font-mono text-[13px] leading-relaxed text-muted">
              <code>
                <span className="text-text">$ karbird context --minutes 30</span>
                {"\n\n"}
                {"# MCP (stdio), read-only token\n"}
                <span className="text-text">$ karbird mcp</span>
              </code>
            </pre>
            <Link href="/integrations" className="mt-6 inline-block text-sm font-semibold underline decoration-border-strong hover:decoration-text">
              Integrations
            </Link>
          </div>
        </div>
      </section>

      <section aria-labelledby="trust" className="border-t border-border bg-bg-2">
        <div className="mx-auto max-w-7xl px-6 py-20">
          <h2 id="trust" className="max-w-2xl text-3xl font-bold tracking-[-0.02em] text-balance">
            Built to be trusted with production telemetry.
          </h2>
          <dl className="mt-10 grid gap-x-10 gap-y-8 sm:grid-cols-2 lg:grid-cols-4">
            {trust.map((t) => (
              <div key={t.title} className="border-t border-border-strong pt-5">
                <dt className="font-semibold">{t.title}</dt>
                <dd className="mt-2 text-sm leading-relaxed text-muted">{t.body}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>

      <section aria-labelledby="start" className="border-t border-border">
        <div className="mx-auto flex max-w-7xl flex-col items-start gap-8 px-6 py-24 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <h2 id="start" className="text-[2.25rem] leading-tight font-bold tracking-[-0.02em] text-balance">
              Put your system on the map.
            </h2>
            <p className="mt-3 max-w-lg leading-relaxed text-muted">Create an account, connect one service and watch it appear live.</p>
          </div>
          <div className="flex flex-wrap items-center gap-4">
            <Link href="/app/register" className="btn btn-primary h-11 px-5 text-sm">
              Start with one service
            </Link>
            <Link href="/app/login" className="text-sm font-semibold underline decoration-border-strong hover:decoration-text">
              Sign in
            </Link>
          </div>
        </div>
      </section>
    </>
  );
}
