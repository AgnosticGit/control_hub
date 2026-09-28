"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import type { Diagnostics, NodeDiff } from "@controlhub/web-engine";
import { useLogout, useSession } from "@/lib/session";
import { formatClock, parseNs } from "@/lib/time";
import { BottomPanel, type Tab } from "./BottomPanel";
import { Inspector } from "./Inspector";
import { MapView } from "./MapView";
import { TimelineBar } from "./TimelineBar";
import { useDebounced } from "./useDebounced";
import { useWorkspaceEngine, type EngineState } from "./useWorkspaceEngine";

export function Workspace({ projectId, environmentId }: { projectId: string; environmentId: string }) {
  const session = useSession();
  const logout = useLogout();
  const { state, actions } = useWorkspaceEngine(projectId, environmentId);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("logs");
  const [traceId, setTraceId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [hideInferred, setHideInferred] = useState(false);
  const [onlyProblems, setOnlyProblems] = useState(false);
  const [tableView, setTableView] = useState(false);
  const [markA, setMarkA] = useState<bigint | null>(null);
  const [compare, setCompare] = useState<{ a: bigint; b: bigint; diffs: NodeDiff[] } | null>(null);
  const debouncedSearch = useDebounced(search, 150);

  const project = session.data?.projects.find((p) => p.id === projectId);
  const env = project?.environments.find((e) => e.id === environmentId);

  useEffect(() => {
    if (state.phase !== "ready") return;
    actions.setFilters({
      search: debouncedSearch.trim(),
      hide_inferred: hideInferred,
      only_problems: onlyProblems,
      selected_trace_id: traceId,
    });
  }, [actions, state.phase, debouncedSearch, hideInferred, onlyProblems, traceId]);

  if (session.isPending) return <p className="p-8 text-sm text-muted">Loading…</p>;
  if (session.data && !env) {
    return (
      <main className="p-8 text-sm">
        <p>This environment does not exist or you do not have access to it.</p>
        <Link href="/app" className="mt-4 inline-block text-accent">
          Back to environments
        </Link>
      </main>
    );
  }

  return (
    <div className="grid h-screen grid-rows-[auto_minmax(0,1fr)_auto] overflow-hidden">
      <header className="flex flex-wrap items-center gap-3 border-b border-border bg-surface px-4 py-2">
        <Link href="/app" className="flex items-center gap-2 font-semibold">
          <span aria-hidden className="inline-block h-2.5 w-2.5 rounded-full bg-accent" />
          ControlHub
        </Link>
        <span className="text-sm text-muted">
          {project?.name ?? projectId} / {env?.name ?? environmentId}
        </span>
        {env?.is_demo ? (
          <span className="rounded border border-warn/50 bg-warn/10 px-2 py-0.5 text-xs font-semibold text-warn" title="Demo environment with synthetic load and fault injection">
            DEMO
          </span>
        ) : null}
        <input
          type="search"
          placeholder="Find component…"
          aria-label="Find component on map"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="ml-4 w-56 rounded border border-border bg-bg px-2 py-1 text-sm"
        />
        <label className="flex items-center gap-1 text-xs text-muted">
          <input type="checkbox" checked={onlyProblems} onChange={(e) => setOnlyProblems(e.target.checked)} />
          Only problems
        </label>
        <label className="flex items-center gap-1 text-xs text-muted">
          <input type="checkbox" checked={hideInferred} onChange={(e) => setHideInferred(e.target.checked)} />
          Hide inferred
        </label>
        <label className="flex items-center gap-1 text-xs text-muted">
          <input type="checkbox" checked={tableView} onChange={(e) => setTableView(e.target.checked)} />
          Table view
        </label>
        <div className="ml-auto flex items-center gap-3">
          <CompletenessBadge state={state} />
          <EngineInfo state={state} diagnostics={actions.diagnostics} />
          <span className="text-xs text-muted">{session.data?.user.email}</span>
          <button type="button" onClick={() => void logout()} className="rounded border border-border px-2 py-1 text-xs hover:border-accent">
            Sign out
          </button>
        </div>
      </header>

      <div className="grid min-h-0 grid-cols-[minmax(0,1fr)_340px] grid-rows-[minmax(0,1fr)_38%]">
        <main className="relative min-h-0 border-r border-border" aria-label="System map">
          {state.phase === "error" ? (
            <div role="alert" className="p-6 text-sm text-error">
              Engine failed: {state.error}
            </div>
          ) : (
            <MapView actions={actions} topology={state.topology} selectedId={selectedId} onSelect={setSelectedId} forceTable={tableView} />
          )}
          {state.phase === "starting" ? (
            <div className="absolute inset-0 flex items-center justify-center text-sm text-muted">Starting engine…</div>
          ) : null}
          {state.phase === "ready" && state.topology && state.topology.nodes.length === 0 && !state.loading ? (
            <div className="absolute inset-0 flex items-center justify-center p-6 text-center text-sm text-muted">
              No telemetry in this time range yet. Start the demo with <code className="mx-1 font-mono">just demo</code> or
              connect a service.
            </div>
          ) : null}
          {state.error && state.phase === "ready" ? (
            <div role="alert" className="absolute left-3 top-16 max-w-md rounded border border-error/50 bg-surface px-3 py-2 text-xs text-error">
              {state.error}
            </div>
          ) : null}
        </main>
        <aside className="row-span-2 min-h-0 overflow-auto bg-surface" aria-label="Inspector">
          <Inspector
            state={state}
            actions={actions}
            selectedId={selectedId}
            onSelect={setSelectedId}
            compare={compare}
            onCloseCompare={() => setCompare(null)}
          />
        </aside>
        <div className="min-h-0 border-r border-border">
          <BottomPanel
            projectId={projectId}
            environmentId={environmentId}
            state={state}
            selectedId={selectedId}
            tab={tab}
            onTab={setTab}
            traceId={traceId}
            onTrace={setTraceId}
          />
        </div>
      </div>

      <TimelineBar
        state={state}
        actions={actions}
        markA={markA}
        onMarkA={setMarkA}
        onCompare={(a, b) => {
          void actions.compare(a, b).then((diffs) => setCompare({ a, b, diffs }));
        }}
      />
    </div>
  );
}

function CompletenessBadge({ state }: { state: EngineState }) {
  const c = state.load?.completeness;
  if (!state.load) return null;
  const problems: string[] = [];
  if (state.load.truncated) problems.push("Timeline truncated: more data exists than loaded.");
  if (c && !c.processing_caught_up) problems.push("Processing is behind ingestion.");
  if (c && c.unresolved_processing_errors > 0) problems.push(`${c.unresolved_processing_errors} record(s) failed processing.`);
  const watermark = parseNs(c?.watermark_ns);
  const title = [
    ...problems,
    ...(c?.notes ?? []),
    watermark ? `Processed up to ${formatClock(watermark)}` : "No processing watermark yet",
    `Sampling: ${c?.sampling ?? "unknown"}`,
  ].join("\n");
  return (
    <span
      title={title}
      className={`rounded border px-2 py-0.5 text-xs ${problems.length ? "border-warn/50 text-warn" : "border-border text-muted"}`}
    >
      {problems.length ? "Data may be incomplete" : "Data complete for range"}
    </span>
  );
}

function EngineInfo({ state, diagnostics }: { state: EngineState; diagnostics: () => Promise<Diagnostics | null> }) {
  const [open, setOpen] = useState(false);
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  return (
    <div className="relative">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => {
          setOpen((v) => !v);
          void diagnostics().then(setDiag);
        }}
        className="rounded border border-border px-2 py-1 text-xs text-muted hover:border-accent"
      >
        Engine
      </button>
      {open ? (
        <div className="absolute right-0 z-10 mt-1 w-72 rounded-md border border-border bg-surface p-3 font-mono text-[11px] shadow-xl">
          <Row k="WASM init" v={state.init ? `${state.init.initMs.toFixed(0)} ms` : "—"} />
          <Row k="WASM size" v={state.init ? `${(state.init.wasmBytes / 1024).toFixed(0)} KiB` : "—"} />
          <Row k="Last frame" v={`${state.frameMs.toFixed(2)} ms`} />
          <Row k="Last fetch / apply" v={state.load ? `${state.load.fetchMs.toFixed(0)} / ${state.load.applyMs.toFixed(1)} ms` : "—"} />
          <Row k="Last batch items" v={String(state.load?.items ?? 0)} />
          {diag ? (
            <>
              <Row k="Spans / logs / events" v={`${diag.spans} / ${diag.logs} / ${diag.events}`} />
              <Row k="Nodes / edges" v={`${diag.nodes} / ${diag.edges}`} />
              <Row k="Checkpoints" v={`${diag.checkpoints} (every ${diag.checkpoint_interval})`} />
              <Row k="Approx memory" v={`${(diag.approx_memory_bytes / 1048576).toFixed(1)} MiB`} />
              <Row k="Duplicates ignored" v={String(diag.duplicates_ignored)} />
              <Row k="Evicted" v={String(diag.evicted_items)} />
              <Row k="Unresolved parents" v={String(diag.unresolved_parents)} />
              <Row k="Stale seeks dropped" v={String(diag.stale_seeks_ignored)} />
            </>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between gap-2">
      <span className="text-muted">{k}</span>
      <span>{v}</span>
    </div>
  );
}
