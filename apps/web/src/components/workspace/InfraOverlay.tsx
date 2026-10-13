"use client";

import { useEffect, useRef, useState } from "react";
import type { InfrastructureSnapshot } from "@karbird/web-engine";
import { Icon } from "@/components/ui/Icon";
import { hintText, hostLabel } from "./infrastructure";

interface Props {
  snapshot: InfrastructureSnapshot | null;
  error: string | null;
  selectedHost: string | null;
  visibleHostIds: string[];
  onlyProblems: boolean;
  onSelectHost: (hostId: string) => void;
  onSelect: (serviceId: string) => void;
  onBottom: (bottom: number) => void;
}

/** Machines and hints float over the map; the renderer reserves their height when fitting cards. */
export function InfraOverlay({ snapshot, error, selectedHost, visibleHostIds, onlyProblems, onSelectHost, onSelect, onBottom }: Props) {
  const [collapsed, setCollapsed] = useState(false);
  const panelRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const panel = panelRef.current;
    const viewport = panel?.parentElement;
    if (!panel || !viewport) return;
    const update = () => onBottom(Math.max(0, Math.ceil(panel.getBoundingClientRect().bottom - viewport.getBoundingClientRect().top)));
    const observer = new ResizeObserver(update);
    observer.observe(panel);
    update();
    return () => {
      observer.disconnect();
      onBottom(0);
    };
  }, [onBottom]);
  const visible = new Set(visibleHostIds);
  const hosts = (snapshot?.hosts ?? []).filter((host) => visible.has(host.id));
  const hints = (snapshot?.hints ?? []).filter((hint) => visible.has(hint.host_id));
  return (
    <div className="pointer-events-none absolute inset-0 z-10">
      <div ref={panelRef} className="pointer-events-auto absolute top-12 left-3 flex max-h-[40%] max-w-[min(28rem,60%)] flex-col gap-2">
        {hints.map((h) => (
          <div
            key={`${h.instance_id}-${h.neighbor_container_id}`}
            role="status"
            aria-label="Noisy neighbour"
            className="pointer-events-auto flex shrink-0 items-start gap-2 rounded-lg border border-warn/50 bg-surface/95 px-3 py-2 text-xs backdrop-blur"
          >
            <Icon name="alert" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warn" />
            <span className="min-w-0 flex-1">{hintText(h)}</span>
            <button type="button" className="shrink-0 text-accent hover:underline" onClick={() => onSelect(h.service_id)}>
              Show
            </button>
          </div>
        ))}
        <section
          aria-label="Machines"
          className="pointer-events-auto flex min-h-0 flex-col rounded-lg border border-border bg-surface/90 px-2 py-1.5 text-xs backdrop-blur"
        >
          <header className="flex shrink-0 items-center gap-2 px-1 pb-1">
            <h2 className="flex-1 text-[11px] text-muted">Machines</h2>
            <button
              type="button"
              className="flex h-6 w-6 items-center justify-center rounded-md text-muted hover:bg-surface-2 hover:text-text"
              aria-label={collapsed ? "Expand machine list" : "Collapse machine list"}
              aria-expanded={!collapsed}
              onClick={() => setCollapsed((value) => !value)}
            >
              <Icon name="chevronDown" className={`h-3 w-3 transition-transform ${collapsed ? "-rotate-90" : "rotate-0"}`} />
            </button>
          </header>
          {!collapsed ? (
            <>
              {error ? (
                <p role="alert" className="px-1 text-error">
                  Infrastructure unavailable: {error}
                </p>
              ) : null}
              {!snapshot && !error ? <p className="px-1 text-muted">Loading…</p> : null}
              {snapshot && hosts.length === 0 ? (
                <p className="px-1 text-muted">
                  {onlyProblems ? "No machines with problem services." : "No machines yet: run the Karbird Collector on your hosts."}
                </p>
              ) : null}
              <ul className="min-h-0 space-y-0.5 overflow-y-auto">
                {hosts.map((h) => (
                  <li key={h.id}>
                    <button
                      type="button"
                      aria-pressed={selectedHost === h.id}
                      onClick={() => onSelectHost(h.id)}
                      className={`flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left hover:bg-surface-2 ${
                        selectedHost === h.id ? "bg-accent/15 text-accent" : ""
                      }`}
                    >
                      <Icon name="cpu" className={`h-3.5 w-3.5 shrink-0 ${h.has_metrics ? "" : "text-muted"}`} />
                      <span className="truncate font-mono">{hostLabel(h)}</span>
                    </button>
                  </li>
                ))}
              </ul>
              {snapshot?.notes.map((n) => (
                <p key={n} className="mt-1 px-1 text-[11px] text-muted">
                  {n}
                </p>
              ))}
            </>
          ) : null}
        </section>
      </div>
    </div>
  );
}
