import Link from "next/link";

const pillars = [
  {
    title: "Standard telemetry in",
    body: "Send OpenTelemetry traces and logs over OTLP (HTTP protobuf or gRPC). No proprietary agent is required.",
  },
  {
    title: "A live map of your system",
    body: "Services and their dependencies are derived from real calls. Every edge says whether it was observed or inferred — and why.",
  },
  {
    title: "Rewind any moment",
    body: "Pause, scrub and replay recent history. State is rebuilt deterministically from recorded data, with completeness shown explicitly.",
  },
  {
    title: "Safe by default",
    body: "Tenant isolation on every query, scoped ingestion tokens, redaction of secrets at ingestion. Telemetry is treated as untrusted input.",
  },
];

export default function HomePage() {
  return (
    <>
      <section className="mx-auto max-w-6xl px-6 pb-16 pt-20">
        <p className="mb-4 text-sm font-medium uppercase tracking-widest text-accent">Observability · Replay · Control</p>
        <h1 className="max-w-3xl text-4xl font-semibold leading-tight tracking-tight sm:text-5xl">
          See what your system is doing — and what it was doing five minutes ago.
        </h1>
        <p className="mt-6 max-w-2xl text-lg text-muted">
          ControlHub turns OpenTelemetry traces, logs and events into a live service map with a timeline you can pause,
          scrub and replay.
        </p>
        <div className="mt-8 flex gap-4">
          <Link href="/getting-started" className="rounded-md bg-accent px-5 py-2.5 font-medium text-bg hover:bg-accent/90">
            Run it locally
          </Link>
          <Link href="/features" className="rounded-md border border-border px-5 py-2.5 font-medium hover:bg-surface">
            How it works
          </Link>
        </div>
      </section>
      <section aria-labelledby="pillars" className="mx-auto max-w-6xl px-6 pb-24">
        <h2 id="pillars" className="sr-only">
          Key capabilities
        </h2>
        <div className="grid gap-4 sm:grid-cols-2">
          {pillars.map((p) => (
            <article key={p.title} className="rounded-lg border border-border bg-surface p-6">
              <h3 className="text-lg font-medium">{p.title}</h3>
              <p className="mt-2 text-sm leading-relaxed text-muted">{p.body}</p>
            </article>
          ))}
        </div>
      </section>
    </>
  );
}
