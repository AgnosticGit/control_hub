"use client";

import { useEffect, useState } from "react";
import type { InfrastructureSnapshot } from "@karbird/web-engine";
import { infraAt } from "./infrastructure";
import { useDebounced } from "./useDebounced";
import type { Mode, WorkspaceActions } from "./useWorkspaceEngine";
import { nowNs } from "@/lib/time";

export interface InfrastructureState {
  snapshot: InfrastructureSnapshot | null;
  error: string | null;
  loading: boolean;
}

/** Loads machines and instances at the selected time while the infrastructure layer is on. */
export function useInfrastructure(actions: WorkspaceActions, scopeKey: string, ready: boolean, enabled: boolean, mode: Mode, cursor: bigint): InfrastructureState {
  const [state, setState] = useState<InfrastructureState & { scopeKey: string; requestKey: string }>({ snapshot: null, error: null, loading: false, scopeKey: "", requestKey: "" });
  const at = useDebounced(infraAt(cursor), 300);
  const cursorKey = mode === "live" ? 0n : at;
  const requestKey = `${scopeKey}|${mode}|${cursorKey}`;
  useEffect(() => {
    if (!ready || !enabled) return;
    let cancelled = false;
    const accept = (snapshot: InfrastructureSnapshot | null) => {
      if (cancelled) return;
      setState((current) => ({
        snapshot: snapshot ?? (current.scopeKey === scopeKey ? current.snapshot : null),
        error: null,
        loading: false,
        scopeKey,
        requestKey,
      }));
    };
    if (mode === "live") {
      const unsubscribe = actions.onInfrastructure(accept);
      void actions.loadInfrastructure(nowNs(), true).then(accept).catch((e: unknown) => {
        if (!cancelled) setState((s) => ({ snapshot: s.scopeKey === scopeKey ? s.snapshot : null, error: e instanceof Error ? e.message : String(e), loading: false, scopeKey, requestKey }));
      });
      return () => { cancelled = true; unsubscribe(); };
    }
    void actions.loadInfrastructure(cursorKey).then(accept).catch((e: unknown) => {
      if (!cancelled) setState((s) => ({ snapshot: s.scopeKey === scopeKey ? s.snapshot : null, error: e instanceof Error ? e.message : String(e), loading: false, scopeKey, requestKey }));
    });
    return () => { cancelled = true; };
  }, [actions, scopeKey, requestKey, ready, enabled, mode, cursorKey]);
  if (!ready || !enabled) return { snapshot: null, error: null, loading: false };
  return {
    snapshot: state.scopeKey === scopeKey ? state.snapshot : null,
    error: state.scopeKey === scopeKey ? state.error : null,
    loading: state.requestKey !== requestKey,
  };
}
