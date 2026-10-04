"use client";

import { useEffect, useRef } from "react";
import type { InfrastructureSnapshot } from "@karbird/web-engine";
import { Icon } from "@/components/ui/Icon";
import { hintText, hostLabel } from "./infrastructure";

interface Props {
  snapshot: InfrastructureSnapshot | null;
  error: string | null;
  selectedHost: string | null;
  onSelectHost: (hostId: string) => void;
  onSelect: (serviceId: string) => void;
  /** How far down the map the overlay reaches, in pixels (0 once it is gone). */
  onBottom?: (px: number) => void;
}

/** Machines of the infrastructure layer and its noisy-neighbour hints, above the map canvas. */
export function InfraOverlay({ snapshot, error, selectedHost, onSelectHost, onSelect, onBottom }: Props) {
  const hosts = snapshot?.hosts ?? [];
  const rootRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = rootRef.current;
    if (!el || !onBottom) return;
    const report = () => onBottom(el.offsetTop + el.offsetHeight);
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    return () => {
      ro.disconnect();
      onBottom(0);
    };
  }, [onBottom]);
  return (
    <div ref={rootRef} className="pointer-events-none absolute top-3 left-3 z-10 flex max-w-[min(28rem,60%)] flex-col gap-2">
      {snapshot?.hints.map((h) => (
        <div
          key={`${h.instance_id}-${h.neighbor_container_id}`}
          role="status"
          aria-label="Noisy neighbour"
          className="pointer-events-auto flex items-start gap-2 rounded-lg border border-warn/50 bg-surface/95 px-3 py-2 text-xs backdrop-blur"
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
        className="pointer-events-auto rounded-lg border border-border bg-surface/90 px-2 py-1.5 text-xs backdrop-blur"
      >
        <h2 className="px-1 pb-1 text-[11px] text-muted">Machines</h2>
        {error ? (
          <p role="alert" className="px-1 text-error">
            Infrastructure unavailable: {error}
          </p>
        ) : null}
        {!snapshot && !error ? <p className="px-1 text-muted">Loading…</p> : null}
        {snapshot && hosts.length === 0 ? (
          <p className="px-1 text-muted">No machines yet: run the Karbird agent on your hosts.</p>
        ) : null}
        <ul className="space-y-0.5">
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
      </section>
    </div>
  );
}
