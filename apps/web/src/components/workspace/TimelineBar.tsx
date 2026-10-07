"use client";

import { useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { TimelineTracksView } from "./timeline/TimelineTracksView";
import { timelineArchiveNote } from "./logs-query";
import { formatSpan, type View } from "./timeline/scale";
import type { WorkspaceMemory } from "./workspace-memory";
import { SPEEDS, type EngineState, type Mode, type WorkspaceActions } from "./useWorkspaceEngine";
import { NS_PER_MS, formatClock, formatClockMicros, formatDateTime, nowNs } from "@/lib/time";

const MODE_STYLE: Record<Mode, string> = {
  live: "bg-ok/10 text-ok border-ok/40",
  paused: "bg-muted/10 text-text/80 border-muted/50",
  replay: "bg-accent/10 text-accent border-accent/40",
  historical: "bg-violet/10 text-violet border-violet/40",
};

const PRESETS = [
  { label: "Last 1 h", minutes: 60 },
  { label: "Last 6 h", minutes: 360 },
  { label: "Last 24 h", minutes: 1440 },
];

interface Props {
  projectId: string;
  environmentId: string;
  /** Changes with the map filters so the tracks are recomputed. */
  filterKey: string;
  /** Service node IDs switched off by the user. */
  muted: string[];
  onMuted: (ids: string[]) => void;
  /** Time span of the focused request: the tracks zoom to it whenever it changes. */
  focus?: View | null;
  /** Height of the whole dock in px (resizable by the caller). */
  height: number;
  state: EngineState;
  actions: WorkspaceActions;
  markA: bigint | null;
  onMarkA: (t: bigint | null) => void;
  onCompare: (a: bigint, b: bigint) => void;
  memory?: WorkspaceMemory;
}

export function TimelineBar({ projectId, environmentId, filterKey, muted, onMuted, focus = null, height, state, actions, markA, onMarkA, onCompare, memory }: Props) {
  const [showRange, setShowRange] = useState(false);
  const playing = state.mode === "replay";
  const summary = state.summary;
  const fine = state.speed < 0.1 && state.mode !== "live";
  const speedIndex = Math.max(0, SPEEDS.indexOf(state.speed));

  return (
    <section className="card relative flex min-h-0 shrink-0 flex-col gap-2 px-3 pt-2 pb-3" style={{ height }} aria-label="Timeline">
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={`w-20 rounded-md border px-2 py-0.5 text-center text-[11px] font-bold tracking-widest uppercase ${MODE_STYLE[state.mode]}`}
          aria-live="polite"
        >
          {state.mode}
        </span>
        <div className="flex items-center gap-0.5 rounded-xl border border-border bg-bg/60 p-0.5">
          <button type="button" className="btn btn-icon btn-sm border-transparent" onClick={actions.stepPrev} disabled={!summary?.prev_event_ns} aria-label="Previous event" title="Previous event">
            <Icon name="skipBack" className="h-4 w-4" />
          </button>
          <button type="button" className="btn btn-icon btn-sm border-transparent" onClick={() => actions.jump(-5000)} aria-label="Back 5 seconds" title="Back 5 s">
            <Icon name="rewind" className="h-4 w-4" />
          </button>
          {playing ? (
            <button type="button" className="btn btn-icon btn-sm btn-primary" onClick={actions.pause} aria-label="Pause" title="Pause">
              <Icon name="pause" className="h-4 w-4" />
            </button>
          ) : (
            <button type="button" className="btn btn-icon btn-sm btn-primary" onClick={() => void actions.play()} aria-label="Play replay" title="Play">
              <Icon name="play" className="h-4 w-4" />
            </button>
          )}
          <button type="button" className="btn btn-icon btn-sm border-transparent" onClick={() => actions.jump(5000)} aria-label="Forward 5 seconds" title="Forward 5 s">
            <Icon name="forward" className="h-4 w-4" />
          </button>
          <button type="button" className="btn btn-icon btn-sm border-transparent" onClick={actions.stepNext} disabled={!summary?.next_event_ns} aria-label="Next event" title="Next event">
            <Icon name="skipForward" className="h-4 w-4" />
          </button>
        </div>
        <div
          className="flex items-center gap-2 rounded-lg border border-border bg-bg/50 px-2 py-1"
          role="group"
          aria-label="Replay speed control"
          title={state.mode === "live" ? "Speed applies to replay; live always follows the wall clock." : "Choose one of seven replay speeds."}
        >
          <span className="text-xs text-muted">Speed</span>
          <output className="min-w-12 rounded-md bg-surface px-1.5 py-0.5 text-center font-mono text-xs font-semibold tabular-nums text-text" aria-live="polite">
            {state.speed}×
          </output>
          <div className="flex w-36 flex-col gap-0.5">
            <input
              type="range"
              min={0}
              max={SPEEDS.length - 1}
              step={1}
              value={speedIndex}
              aria-label="Replay speed"
              aria-valuetext={`${state.speed}×`}
              aria-describedby="replay-speed-range"
              onChange={(event) => {
                const selected = SPEEDS[Number(event.target.value)];
                if (selected !== undefined) actions.setSpeed(selected);
              }}
              className="h-4 w-full cursor-pointer accent-accent"
            />
            <span id="replay-speed-range" className="flex justify-between text-[9px] leading-none text-muted">
              <span>0.001×</span>
              <span>10×</span>
            </span>
          </div>
        </div>
        {state.mode === "live" ? (
          <button type="button" className="btn btn-sm" onClick={actions.pause}>
            <Icon name="pause" className="h-3.5 w-3.5" />
            Pause live
          </button>
        ) : (
          <button type="button" className="btn btn-sm btn-ok" onClick={() => void actions.returnToLive()}>
            <Icon name="radio" className="h-3.5 w-3.5" />
            Return to live
          </button>
        )}
        <span className="rounded-lg border border-border bg-bg/60 px-2.5 py-0.5 font-mono text-sm tabular-nums" title={formatDateTime(state.cursor)}>
          {fine ? formatClockMicros(state.cursor) : formatClock(state.cursor)}
          {fine ? <span className="ml-1 text-[10px] text-muted">µs</span> : null}
        </span>
        {state.loop ? (
          <span
            className="flex items-center gap-1.5 rounded-lg border border-accent-2/50 bg-accent-2/10 py-0.5 pr-0.5 pl-2 font-mono text-[11px] text-accent-2 tabular-nums"
            title="Play loops inside this range until it is cleared (Esc on the timeline)"
          >
            <Icon name="repeat" className="h-3 w-3" />
            <span aria-label="Replay range">
              {formatClock(state.loop.from)} – {formatClock(state.loop.to)} · {formatSpan(state.loop.to - state.loop.from)}
            </span>
            <button type="button" className="tl-btn text-accent-2" onClick={() => actions.setLoop(null)} aria-label="Clear replay range" title="Clear the range">
              <Icon name="x" className="h-3 w-3" />
            </button>
          </span>
        ) : null}
        <span className="hidden font-mono text-[11px] text-muted lg:inline" title="At the selected time; counts are for the 30 s window ending there">
          {summary ? `${summary.total_active} active · ${summary.window_requests} req · ${summary.window_errors} err` : ""}
        </span>
        {state.load?.truncated ? <span className="text-[11px] text-warn">Loaded data is truncated: tracks may be incomplete.</span> : null}
        {timelineArchiveNote(state.load?.logsSource) ? (
          <span
            className="rounded-md border border-violet/40 bg-violet/10 px-1.5 py-0.5 text-[11px] text-violet"
            title="Logs older than the hot window come from the long-term archive. Traces and events are not archived, so requests and the map may be empty there."
          >
            {timelineArchiveNote(state.load?.logsSource)}
          </span>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          <span className="hidden text-[11px] text-muted 2xl:inline">Ruler: move in time · Drag tracks: replay range · Ctrl+wheel: zoom</span>
          {markA === null ? (
            <button type="button" className="btn btn-sm" onClick={() => onMarkA(state.cursor)} title="Mark this moment as A for comparison">
              <Icon name="swap" className="h-3.5 w-3.5" />
              Mark A
            </button>
          ) : (
            <>
              <span className="text-xs text-muted">A: {formatClock(markA)}</span>
              <button type="button" className="btn btn-sm" onClick={() => onCompare(markA, state.cursor)}>
                Compare A ↔ now
              </button>
              <button type="button" className="btn btn-icon btn-sm" onClick={() => onMarkA(null)} aria-label="Clear mark A">
                <Icon name="x" className="h-3.5 w-3.5" />
              </button>
            </>
          )}
          <button type="button" className="btn btn-sm" onClick={() => setShowRange((v) => !v)} aria-expanded={showRange}>
            <Icon name="calendar" className="h-3.5 w-3.5" />
            Range…
          </button>
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

      <TimelineTracksView
        projectId={projectId}
        environmentId={environmentId}
        state={state}
        actions={actions}
        markA={markA}
        filterKey={filterKey}
        muted={muted}
        onMuted={onMuted}
        focus={focus}
        memory={memory}
      />
    </section>
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
    <div className="absolute right-3 bottom-full z-30 mb-2 flex flex-wrap items-end gap-3 rounded-xl border border-border-strong bg-surface-2 p-3 text-xs shadow-2xl">
      {PRESETS.map((p) => (
        <button
          key={p.label}
          type="button"
          className="btn btn-sm"
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
        <input type="datetime-local" value={from} onChange={(e) => setFrom(e.target.value)} className="input h-8" />
      </label>
      <label className="flex flex-col gap-1 text-muted">
        To
        <input type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} className="input h-8" />
      </label>
      <button
        type="button"
        className="btn btn-primary btn-sm"
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
      <p className="basis-full text-[11px] text-muted">
        Up to 7 days at a time. Logs older than about two weeks come from the archive (kept for a year); traces and events
        are not archived.
      </p>
      {error ? (
        <span role="alert" className="text-error">
          {error}
        </span>
      ) : null}
    </div>
  );
}
