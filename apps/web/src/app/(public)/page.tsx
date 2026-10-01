import Link from "next/link";
import { MapDemo } from "@/components/landing/MapDemo";

const legend = [
  {
    swatch: <span className="block h-0.5 w-10 rounded-full bg-accent" />,
    title: "Observed",
    body: "Drawn from real spans; the line gets heavier with call volume and moving dots are recorded requests.",
  },
  {
    swatch: <span className="block h-0 w-10 border-t-2 border-dashed border-[#475569]" />,
    title: "Inferred",
    body: "A dependency we have reason to believe in but have not seen in spans, always with the reason.",
  },
  {
    swatch: <span className="block h-4 w-8 rounded-md border-[1.5px] border-error bg-surface" />,
    title: "Failing",
    body: "Services and requests that error, with correlated logs and the trace one click away.",
  },
  {
    swatch: <span className="block h-4 w-0.5 bg-ok" />,
    title: "Deploys and actions",
    body: "Changes sit on the same timeline as the telemetry, so cause and effect line up.",
  },
];

const pipeline = ["Your services", "Ingest", "Kafka", "Processor", "ClickHouse", "API", "Engine in your browser"];

const control = [
  { name: "Plan", note: "a typed action from the catalog" },
  { name: "Policy", note: "checked before anyone approves" },
  { name: "Approval", note: "by a second person" },
  { name: "Executor", note: "a separate process applies it" },
  { name: "Verified", note: "from telemetry afterwards" },
  { name: "Audit", note: "every step on the record" },
];

const TELEMETRY = "#38bdf8";
const CONTROL = "#f472b6";
const API_INDEX = pipeline.indexOf("API");

/** One stop on a vertical line; `interchange` draws the larger ring where two lines meet. */
function Stop({ name, note, interchange, terminus }: { name: string; note?: string; interchange?: boolean; terminus?: boolean }) {
  return (
    <li className="relative flex h-16 gap-4">
      <span
        aria-hidden
        className={`relative z-[1] mt-0 block shrink-0 rounded-full border-text bg-bg ${interchange ? "-mx-[3px] h-6 w-6 border-4" : terminus ? "h-[18px] w-[18px] border-[5px]" : "h-[18px] w-[18px] border-[3px]"}`}
      />
      <span className={interchange ? "-mt-px sm:mt-6" : "-mt-0.5"}>
        <span className="block text-[15px] font-semibold">{name}</span>
        {note ? <span className="mt-0.5 block text-sm text-muted">{note}</span> : null}
      </span>
    </li>
  );
}

/** Telemetry line top to bottom; the control line branches off at the API interchange. */
function SystemLines() {
  return (
    <div className="grid gap-y-10 sm:grid-cols-2 sm:gap-x-10" style={{ ["--api-top" as string]: `${API_INDEX * 4}rem` }}>
      <div className="relative">
        <span aria-hidden className="absolute top-[9px] bottom-[calc(4rem-9px)] left-[7px] w-1 rounded-full" style={{ background: TELEMETRY }} />
        <ol aria-label="Telemetry path">
          {pipeline.map((name, i) => (
            <Stop key={name} name={name} interchange={i === API_INDEX} terminus={i === 0 || i === pipeline.length - 1} />
          ))}
        </ol>
      </div>
      <div className="relative sm:pt-[var(--api-top)]">
        <span
          aria-hidden
          className="absolute top-[calc(var(--api-top)+7px)] left-[calc(-100%-2.5rem+9px)] hidden h-1 w-[calc(100%+2.5rem)] rounded-full sm:block"
          style={{ background: CONTROL }}
        />
        <span aria-hidden className="absolute top-[9px] bottom-[calc(4rem-9px)] left-[7px] w-1 rounded-full sm:top-[calc(var(--api-top)+9px)]" style={{ background: CONTROL }} />
        <ol aria-label="Control path, branching off at the API">
          {control.map((s, i) => (
            <Stop key={s.name} name={s.name} note={s.note} terminus={i === control.length - 1} />
          ))}
        </ol>
      </div>
    </div>
  );
}

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
              ControlHub turns OpenTelemetry traces, logs and events into a live service map with a timeline you can pause, scrub and
              replay. Changes go through plans, approval and verification.
            </p>
            <div className="mt-7 flex flex-wrap items-center gap-3">
              <Link href="/getting-started" className="btn btn-primary h-11 px-5 text-sm">
                Run it locally
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
            The same map and timeline as the workspace, running on a synthetic system. OpenTelemetry in · No proprietary agent · Early
            development build
          </p>
        </div>
      </section>

      <section aria-labelledby="legend" className="border-t border-border">
        <div className="mx-auto grid max-w-7xl gap-12 px-6 py-20 lg:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
          <div>
            <h2 id="legend" className="text-3xl font-bold tracking-[-0.02em] text-balance">
              Every line on the map says how we know.
            </h2>
            <p className="mt-4 max-w-md leading-relaxed text-muted">
              Observed and inferred are never mixed up, and missing data is shown as missing. State at any moment is rebuilt from
              recorded telemetry, so the same moment always replays the same way.
            </p>
          </div>
          <dl className="grid gap-x-10 gap-y-8 sm:grid-cols-2">
            {legend.map((l) => (
              <div key={l.title} className="border-t border-border pt-5">
                <dt className="flex items-center gap-4 font-semibold">
                  <span className="flex h-5 w-10 items-center" aria-hidden>
                    {l.swatch}
                  </span>
                  {l.title}
                </dt>
                <dd className="mt-2 text-sm leading-relaxed text-muted">{l.body}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>

      <section aria-labelledby="pipeline" className="border-t border-border bg-bg-2">
        <div className="mx-auto max-w-7xl px-6 py-20">
          <div className="grid gap-10 sm:grid-cols-2 sm:gap-x-10">
            <div>
              <h2 id="pipeline" className="flex items-start gap-4 text-3xl font-bold tracking-[-0.02em] text-balance">
                <span aria-hidden className="mt-3.5 h-1.5 w-8 shrink-0 rounded-full" style={{ background: TELEMETRY }} />
                Standard telemetry in, nothing dropped on the way.
              </h2>
              <p className="mt-4 max-w-md leading-relaxed text-muted sm:ml-12">
                Send OTLP over HTTP or gRPC. Ingest acknowledges only after the broker has the data; if it cannot, your exporter gets an
                error, not a silent success. Secrets are redacted at ingestion and every query is scoped to your tenant.
              </p>
            </div>
            <div>
              <h2 id="control" className="flex items-start gap-4 text-3xl font-bold tracking-[-0.02em] text-balance">
                <span aria-hidden className="mt-3.5 h-1.5 w-8 shrink-0 rounded-full" style={{ background: CONTROL }} />
                Nothing touches your system without a plan and a second pair of eyes.
              </h2>
              <p className="mt-4 max-w-md leading-relaxed text-muted sm:ml-12">
                Actions are typed and bounded, never free-form commands. Today they run against the demo system only; real adapters come
                later, each with its own allowlist and verification.
              </p>
            </div>
          </div>
          <div className="mt-14">
            <SystemLines />
          </div>
        </div>
      </section>

      <section aria-labelledby="assistants" className="border-t border-border bg-bg-2">
        <div className="mx-auto grid max-w-7xl items-center gap-12 px-6 py-20 lg:grid-cols-2">
          <div>
            <h2 id="assistants" className="text-3xl font-bold tracking-[-0.02em] text-balance">
              Give your coding assistant the real picture, read-only.
            </h2>
            <p className="mt-4 max-w-lg leading-relaxed text-muted">
              The <code className="font-mono text-[0.9em] text-text">controlhub</code> CLI and its MCP server hand an assistant the
              topology, errors and recent history of one environment, with a read-only token. It can look; it cannot change anything.
            </p>
            <Link href="/integrations" className="mt-6 inline-block text-sm font-semibold text-text underline decoration-border-strong hover:decoration-text">
              Integrations
            </Link>
          </div>
          <pre className="overflow-x-auto rounded-lg border border-border bg-bg p-5 font-mono text-[13px] leading-relaxed text-muted">
            <code>
              <span className="text-text">$ controlhub discover</span>
              {"\n"}
              <span className="text-text">$ controlhub context --minutes 30</span>
              {"\n\n"}
              {"# MCP (stdio), read-only token\n"}
              <span className="text-text">$ controlhub mcp</span>
            </code>
          </pre>
        </div>
      </section>

      <section aria-labelledby="start" className="border-t border-border">
        <div className="mx-auto grid max-w-7xl items-center gap-12 px-6 py-24 lg:grid-cols-2">
          <div>
            <h2 id="start" className="text-3xl font-bold tracking-[-0.02em] text-balance">
              Run the whole stand on your machine.
            </h2>
            <p className="mt-4 max-w-lg leading-relaxed text-muted">
              ControlHub is an early development build: no production deployment yet, no metrics ingestion, no archiving. Everything on
              this page works end to end on the local demo stand.
            </p>
            <div className="mt-8 flex flex-wrap gap-4 text-sm">
              <Link href="/getting-started" className="btn btn-primary h-11 px-5 text-sm">
                Set up the demo stand
              </Link>
              <Link href="/features" className="self-center font-semibold underline decoration-border-strong hover:decoration-text">
                What works today
              </Link>
            </div>
          </div>
          <pre className="overflow-x-auto rounded-lg border border-border bg-bg-2 p-5 font-mono text-[13px] leading-relaxed text-text">
            <code>{"just bootstrap\njust wasm\njust dev\njust demo   # second terminal"}</code>
          </pre>
        </div>
      </section>
    </>
  );
}
