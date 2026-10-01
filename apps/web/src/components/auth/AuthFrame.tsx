import Link from "next/link";
import type { ReactNode } from "react";
import { Logo } from "@/components/ui/Icon";

const FACTS = [
  { color: "#38bdf8", text: "A live map of your services, drawn from OpenTelemetry." },
  { color: "#f472b6", text: "A timeline you can pause and scrub back to before the fault." },
  { color: "#34d399", text: "Changes only through plans, a second approval and verification." },
];

/** Static line diagram: three service lines through one interchange. */
function Diagram() {
  return (
    <svg viewBox="0 0 320 180" className="h-auto w-full max-w-sm" aria-hidden>
      <path d="M10 140h90l60-60h150" fill="none" stroke="#38bdf8" strokeWidth="6" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M160 10v160" fill="none" stroke="#f472b6" strokeWidth="6" strokeLinecap="round" />
      <path d="M40 30l70 50h110l60 60" fill="none" stroke="#34d399" strokeWidth="6" strokeLinecap="round" strokeLinejoin="round" />
      {[
        [100, 140],
        [220, 80],
        [160, 30],
        [280, 140],
      ].map(([x, y]) => (
        <circle key={`${x}-${y}`} cx={x} cy={y} r="7" className="fill-bg-2" stroke="#eceef0" strokeWidth="3" />
      ))}
      <circle cx="160" cy="80" r="11" className="fill-bg-2" stroke="#eceef0" strokeWidth="3.5" />
    </svg>
  );
}

export function AuthFrame({ children }: { children: ReactNode }) {
  return (
    <main className="grid min-h-screen lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="flex flex-col px-6 py-8 sm:px-12">
        <Link href="/" className="flex items-center gap-2.5 self-start text-[17px] font-bold tracking-[-0.01em]">
          <Logo />
          ControlHub
        </Link>
        <div className="flex flex-1 items-center justify-center py-12">{children}</div>
      </div>
      <aside className="hidden flex-col justify-center gap-10 border-l border-border bg-bg-2 px-12 lg:flex" aria-label="About ControlHub">
        <Diagram />
        <ul className="max-w-sm space-y-4">
          {FACTS.map((f) => (
            <li key={f.text} className="flex gap-4 text-[15px] leading-snug">
              <span className="mt-1.5 h-1.5 w-6 shrink-0 rounded-full" style={{ background: f.color }} aria-hidden />
              {f.text}
            </li>
          ))}
        </ul>
      </aside>
    </main>
  );
}
