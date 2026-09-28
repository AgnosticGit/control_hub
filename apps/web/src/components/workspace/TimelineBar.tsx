"use client";

import { useState } from "react";
import { SPEEDS, type EngineState, type Mode, type WorkspaceActions } from "./useWorkspaceEngine";
import { NS_PER_MS, formatClock, formatDateTime, nowNs } from "@/lib/time";

const MODE_STYLE: Record<Mode, string> = {
  live: "bg-ok/15 text-ok border-ok/40",
  paused: "bg-warn/15 text-warn border-warn/40",
  replay: "bg-accent/15 text-accent border-accent/40",
  historical: "bg-accent-2/15 text-accent-2 border-accent-2/40",
};

const PRESETS = [
  { label: "Last 1 h", minutes: 60 },
  { label: "Last 6 h", minutes: 360 },
  { label: "Last 24 h", minutes: 1440 },
];

interface Props {
  state: EngineState;
  actions: WorkspaceActions;
  markA: bigint | null;
  onMarkA: (t: bigint | null) => void;
  onCompare: (a: bigint, b: bigint) => void;
}

const SCRUB_STEPS = 10_000;

export function TimelineBar({ state, actions, markA, onMarkA, onCompare }: Props) {
  const [showRange, setShowRange] = useState(false);
  const span = state.rangeTo - state.rangeFrom;
  const pos = span > 0n ? Number(((state.cursor - state.rangeFrom) * BigInt(SCRUB_STEPS)) / span) : SCRUB_STEPS;
  const markPos =
    markA !== null && span > 0n ? Number(((markA - state.rangeFrom) * 100n) / span) : null;
  const playing = state.mode === "replay";
  const summary = state.summary;

  const btn =
    "h-8 min-w-8 rounded-md border border-border bg-surface px-2 text-sm hover:border-accent disabled:opacity-40 disabled:hover:border-border";

  return (
    <div className="border-t border-border bg-surface px-4 py-2" aria-label="Timeline">
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={`rounded border px-2 py-0.5 text-xs font-semibold uppercase tracking-wide ${MODE_STYLE[state.mode]}`}
          aria-live="polite"
        >
          {state.mode}
        </span>
        <button type="button" className={btn} onClick={actions.stepPrev} disabled={!summary?.prev_event_ns} aria-label="Previous event" title="Previous event">
          ⏮
        </button>
        <button type="button" className={btn} onClick={() => actions.jump(-5000)} aria-label="Back 5 seconds" title="Back 5 s">
          −5s
        </button>
        {playing ? (
          <button type="button" className={btn} onClick={actions.pause} aria-label="Pause" title="Pause">
            ⏸
          </button>
        ) : (
          <button type="button" className={btn} onClick={actions.play} aria-label="Play replay" title="Play">
            ▶
          </button>
        )}
        <button type="button" className={btn} onClick={() => actions.jump(5000)} aria-label="Forward 5 seconds" title="Forward 5 s">
          +5s
        </button>
        <button type="button" className={btn} onClick={actions.stepNext} disabled={!summary?.next_event_ns} aria-label="Next event" title="Next event">
          ⏭
        </button>
        <label className="ml-1 flex items-center gap-1 text-xs text-muted">
          Speed
          <select
            value={state.speed}
            onChange={(e) => actions.setSpeed(Number(e.target.value) as (typeof SPEEDS)[number])}
            className="rounded border border-border bg-bg px-1 py-1 text-xs text-text"
          >
            {SPEEDS.map((s) => (
              <option key={s} value={s}>
                {s}×
              </option>
            ))}
          </select>
        </label>
        {state.mode === "live" ? (
          <button type="button" className={btn} onClick={actions.pause}>
            Pause live
          </button>
        ) : (
          <button type="button" className={`${btn} border-ok/50 text-ok`} onClick={() => void actions.returnToLive()}>
            Return to live
          </button>
        )}
        <span className="mx-2 font-mono text-sm tabular-nums" title={formatDateTime(state.cursor)}>
          {formatClock(state.cursor)}
        </span>
        <div className="ml-auto flex items-center gap-2">
          {markA === null ? (
            <button type="button" className={btn} onClick={() => onMarkA(state.cursor)} title="Mark this moment as A for comparison">
              Mark A
            </button>
          ) : (
            <>
              <span className="text-xs text-muted">A: {formatClock(markA)}</span>
              <button type="button" className={btn} onClick={() => onCompare(markA, state.cursor)}>
                Compare A ↔ now
              </button>
              <button type="button" className={btn} onClick={() => onMarkA(null)} aria-label="Clear mark A">
                ✕
              </button>
            </>
          )}
          <button type="button" className={btn} onClick={() => setShowRange((v) => !v)} aria-expanded={showRange}>
            Range…
          </button>
        </div>
      </div>

      <div className="relative mt-2">
        <input
          type="range"
          min={0}
          max={SCRUB_STEPS}
          value={Math.max(0, Math.min(SCRUB_STEPS, pos))}
          aria-label="Selected time"
          aria-valuetext={formatDateTime(state.cursor)}
          onChange={(e) => {
            const p = BigInt(e.target.value);
            actions.seek(state.rangeFrom + (span * p) / BigInt(SCRUB_STEPS));
          }}
          className="w-full accent-[var(--color-accent)]"
        />
        {markPos !== null ? (
          <span
            aria-hidden
            className="pointer-events-none absolute top-0 h-full w-0.5 bg-warn"
            style={{ left: `${markPos}%` }}
          />
        ) : null}
        <div className="flex justify-between font-mono text-[11px] text-muted">
          <span>{formatClock(state.rangeFrom, false)}</span>
          <span>
            {summary ? `${summary.total_active} active · ${summary.window_requests} req · ${summary.window_errors} err (30 s window)` : ""}
          </span>
          <span>{formatClock(state.rangeTo, false)}</span>
        </div>
      </div>

      {showRange ? (
        <RangePicker
          onPick={(from, to) => {
            setShowRange(false);
            void actions.loadHistorical(from, to);
          }}
        />
      ) : null}
    </div>
  );
}

function toLocalInput(ns: bigint) {
  const d = new Date(Number(ns / NS_PER_MS));
  const off = d.getTimezoneOffset() * 60_000;
  return new Date(d.getTime() - off).toISOString().slice(0, 16);
}

function RangePicker({ onPick }: { onPick: (from: bigint, to: bigint) => void }) {
  const now = nowNs();
  const [from, setFrom] = useState(toLocalInput(now - 3_600_000n * NS_PER_MS));
  const [to, setTo] = useState(toLocalInput(now));
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="mt-2 flex flex-wrap items-end gap-3 rounded-md border border-border bg-bg p-3 text-xs">
      {PRESETS.map((p) => (
        <button
          key={p.label}
          type="button"
          className="rounded border border-border px-2 py-1 hover:border-accent"
          onClick={() => {
            const t = nowNs();
            onPick(t - BigInt(p.minutes) * 60_000n * NS_PER_MS, t);
          }}
        >
          {p.label}
        </button>
      ))}
      <label className="flex flex-col gap-1 text-muted">
        From
        <input type="datetime-local" value={from} onChange={(e) => setFrom(e.target.value)} className="rounded border border-border bg-surface px-2 py-1 text-text" />
      </label>
      <label className="flex flex-col gap-1 text-muted">
        To
        <input type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} className="rounded border border-border bg-surface px-2 py-1 text-text" />
      </label>
      <button
        type="button"
        className="rounded bg-accent px-3 py-1 font-medium text-bg"
        onClick={() => {
          const f = Date.parse(from);
          const t = Date.parse(to);
          if (!Number.isFinite(f) || !Number.isFinite(t) || f >= t) {
            setError("Choose a valid range (from before to).");
            return;
          }
          if (t - f > 7 * 24 * 3_600_000) {
            setError("Range may not exceed 7 days.");
            return;
          }
          onPick(BigInt(f) * NS_PER_MS, BigInt(t) * NS_PER_MS);
        }}
      >
        Load range
      </button>
      {error ? <span role="alert" className="text-error">{error}</span> : null}
    </div>
  );
}
