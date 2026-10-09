"use client";

import { useEffect, useMemo, useState } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { api, scopedPath, unwrap } from "@/lib/api";
import { formatClock, parseNs } from "@/lib/time";
import type { PanelScope } from "./SignalsPanel";
import type { WorkspaceActions } from "./useWorkspaceEngine";
import type { Schemas } from "@/lib/api";
import { eventAttributeText, filterEvents } from "./events-query";

type Event = Schemas["EventDto"];

export function EventsPanel({ scope, actions, onTrace, onSelectRecord }: { scope: PanelScope; actions: WorkspaceActions; onTrace: (id: string) => void; onSelectRecord: (serviceName: string, time: bigint) => void }) {
  const [search, setSearch] = useState("");
  const [eventType, setEventType] = useState("all");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [liveEvents, setLiveEvents] = useState<Event[]>([]);
  const query = useQuery({
    queryKey: ["events", scope.projectId, scope.environmentId, scope.from.toString(), scope.to.toString()],
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/events", {
          params: {
            path: scopedPath(scope.projectId, scope.environmentId),
            query: { from: scope.from.toString(), to: scope.to.toString(), limit: 500 },
          },
        }),
      ),
    placeholderData: keepPreviousData,
    refetchInterval: false,
  });
  useEffect(() => {
    if (!scope.live) return;
    return actions.onLiveSignals((signals) => {
      const incoming = signals.filter((signal) => signal.kind === "event").map((signal) => signal.record as Event);
      if (incoming.length) setLiveEvents((current) => [...current, ...incoming].slice(-2000));
    });
  }, [actions, scope.live]);
  const events = useMemo(() => {
    const base = (query.data?.events ?? []).filter((event) => {
      const time = BigInt(event.time_ns);
      return time >= scope.displayFrom && time < scope.displayTo;
    });
    if (!scope.live) return base;
    const merged = new Map(base.map((event) => [event.id, event]));
    for (const event of liveEvents) {
      const time = BigInt(event.time_ns);
      if (time >= scope.displayFrom && time < scope.displayTo && (!scope.serviceId || event.service_id === scope.serviceId)) merged.set(event.id, event);
    }
    return [...merged.values()].sort((a, b) => (BigInt(a.time_ns) > BigInt(b.time_ns) ? -1 : 1));
  }, [query.data?.events, liveEvents, scope.live, scope.displayFrom, scope.displayTo, scope.serviceId]);
  const types = useMemo(() => [...new Set(events.map((event) => event.event_type))].sort(), [events]);
  const filtered = useMemo(() => filterEvents(events, { query: search, eventType, serviceName: scope.serviceName }), [events, search, eventType, scope.serviceName]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2">
        <input
          type="search"
          placeholder="Search events, services, attributes…"
          aria-label="Search events"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          className="input h-8 min-w-32 flex-1 text-xs"
        />
        <label className="sr-only" htmlFor="event-type-filter">Event type</label>
        <select id="event-type-filter" aria-label="Event type" className="select h-8 max-w-44 text-xs" value={eventType} onChange={(event) => setEventType(event.target.value)}>
          <option value="all">All types</option>
          {types.map((type) => <option key={type} value={type}>{type}</option>)}
        </select>
        <span className="ml-auto text-[11px] text-muted">{query.isFetching ? "Loading… " : ""}{filtered.length} {filtered.length === 1 ? "event" : "events"}</span>
      </div>

      {query.error ? (
        <p role="alert" className="p-3 text-xs text-error">Failed to load events: {query.error.message}</p>
      ) : filtered.length === 0 && !query.isPending ? (
        <div className="m-3 rounded-xl border border-border bg-surface-2 p-4 text-sm">
          <p className="font-medium">{search || eventType !== "all" || scope.serviceName ? "No events match these filters" : "No events in this time range"}</p>
          <p className="mt-1 text-xs text-muted">Try a broader time range, clear the search or choose another event type.</p>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto px-2 py-1" aria-label="Event records">
          <div role="list" aria-label="Events">
            {filtered.map((event) => {
              const time = parseNs(event.time_ns);
              const expanded = expandedId === event.id;
              const attrs = eventAttributeText(event.attributes);
              const traceId = event.trace_id;
              return (
                <article key={event.id} role="listitem" className={`my-1 rounded-xl border bg-surface px-3 py-2 transition-colors ${expanded ? "border-accent/50 bg-surface-2" : "border-border hover:border-border-strong hover:bg-surface-2"}`}>
                  <button
                    type="button"
                    className="block w-full text-left"
                    aria-expanded={expanded}
                    aria-label={`${event.event_type} event from ${event.service_name} at ${time === null ? "unknown time" : formatClock(time)}`}
                    onClick={() => {
                      setExpandedId(expanded ? null : event.id);
                      if (time !== null) onSelectRecord(event.service_name, time);
                    }}
                  >
                    <span className="flex min-w-0 items-center gap-2">
                      <span className="shrink-0 font-mono text-[11px] tabular-nums text-muted">{time === null ? "?" : formatClock(time)}</span>
                      <span className="min-w-0 flex-1 truncate rounded-md border border-accent/25 bg-accent/10 px-1.5 py-0.5 text-[10px] font-semibold text-accent">{event.event_type}</span>
                      <span className="text-[10px] text-muted">{expanded ? "Hide details" : "Details"}</span>
                    </span>
                    <span className="mt-1 block truncate text-xs font-medium text-text">{event.service_name}</span>
                    <span className="mt-1 block truncate text-[11px] text-muted">{attrs || "No attributes"}</span>
                  </button>
                  {expanded ? (
                    <div className="mt-2 border-t border-border pt-2 text-xs">
                      <p className="mb-1 text-[10px] font-semibold tracking-wide text-muted uppercase">Event details</p>
                      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
                        <dt className="text-muted">Type</dt><dd className="break-words">{event.event_type}</dd>
                        <dt className="text-muted">Service</dt><dd className="break-words">{event.service_name}</dd>
                        {Object.entries(event.attributes).sort(([a], [b]) => a.localeCompare(b)).map(([key, value]) => (
                          <div key={key} className="contents"><dt className="text-muted">{key}</dt><dd className="min-w-0 break-all">{value}</dd></div>
                        ))}
                      </dl>
                      {traceId ? <button type="button" className="btn btn-sm mt-2 text-accent" onClick={() => onTrace(traceId)}>Open trace</button> : null}
                      {traceId || event.span_id ? <dl className="mt-2 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 font-mono text-[10px]">
                        {traceId ? <><dt className="text-muted">Trace ID</dt><dd className="break-all">{traceId}</dd></> : null}
                        {event.span_id ? <><dt className="text-muted">Span ID</dt><dd className="break-all">{event.span_id}</dd></> : null}
                      </dl> : null}
                    </div>
                  ) : null}
                </article>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
