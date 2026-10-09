"use client";

import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useRef } from "react";
import { Icon } from "@/components/ui/Icon";
import { api, scopedPath, unwrap } from "@/lib/api";
import { formatSpan, type View } from "./timeline/scale";
import { speedFor, traceRange } from "./trace-focus";
import type { EngineState, WorkspaceActions } from "./useWorkspaceEngine";

interface Props {
  projectId: string;
  environmentId: string;
  traceId: string;
  traceOnly: boolean;
  onTraceOnly: (on: boolean) => void;
  onClose: () => void;
  /** Reports the request's time span so the timeline can zoom to it. */
  onFocus: (range: View | null) => void;
  state: EngineState;
  actions: WorkspaceActions;
}

/**
 * The selected request over the map: shows only it (optionally) and replays it slowly from
 * start to end on a loop, so every call and answer can be followed across services.
 */
export function TraceFocusBar({ projectId, environmentId, traceId, traceOnly, onTraceOnly, onClose, onFocus, state, actions }: Props) {
  const detail = useQuery({
    queryKey: ["trace", projectId, environmentId, traceId],
    queryFn: async () =>
      unwrap(
        await api.GET("/api/v1/projects/{project_id}/environments/{environment_id}/traces/{trace_id}", {
          params: { path: { ...scopedPath(projectId, environmentId), trace_id: traceId } },
        }),
      ),
  });
  const spans = detail.data?.spans;
  const bounds = useMemo<View>(() => ({ from: state.rangeFrom, to: state.rangeTo }), [state.rangeFrom, state.rangeTo]);
  const range = useMemo(() => (spans ? traceRange(spans, bounds) : null), [spans, bounds]);
  const services = useMemo(() => new Set((spans ?? []).map((s) => s.service_id)).size, [spans]);

  const play = () => {
    if (!range) return;
    actions.playRange(range, speedFor(range.to - range.from));
    onFocus(range);
  };

  // Replay a newly selected request once its spans are known.
  const played = useRef<string | null>(null);
  useEffect(() => {
    if (!range || played.current === traceId) return;
    played.current = traceId;
    actions.playRange(range, speedFor(range.to - range.from));
    onFocus(range);
  }, [range, traceId, actions, onFocus]);

  return (
    <div
      role="region"
      aria-label="Focused request"
      className="absolute top-3 left-3 z-10 flex max-w-[calc(100%-22rem)] flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-accent/40 bg-surface/90 px-3 py-2 text-[11px] shadow-lg backdrop-blur"
    >
      <span className="flex items-center gap-1.5 font-medium text-text">
        <Icon name="crosshair" className="h-3.5 w-3.5 text-accent" />
        Request
        <span className="max-w-40 truncate font-mono text-muted" title={traceId}>
          {traceId}
        </span>
      </span>
      {detail.isPending ? <span className="text-muted">loading…</span> : null}
      {detail.error ? <span className="text-error">failed to load: {detail.error.message}</span> : null}
      {spans ? (
        <span className="text-muted" aria-label="Request size">
          {spans.length} spans · {services} services{range ? ` · ${formatSpan(range.to - range.from)}` : ""}
        </span>
      ) : null}
      {spans && !range ? <span className="text-warn">outside the loaded range</span> : null}
      <label className="toggle">
        <input type="checkbox" className="switch" checked={traceOnly} onChange={(e) => onTraceOnly(e.target.checked)} />
        Only this request
      </label>
      <button type="button" className="btn btn-sm" disabled={!range} onClick={play} title="Replay the request at a readable pace">
        <Icon name="play" className="h-3.5 w-3.5" />
        Replay request
      </button>
      <button type="button" className="btn btn-icon btn-sm" aria-label="Stop focusing this request" title="Show all requests again" onClick={onClose}>
        <Icon name="x" className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
