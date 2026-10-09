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
}

/** Loads machines and instances at the selected time while the infrastructure layer is on. */
export function useInfrastructure(actions: WorkspaceActions, scopeKey: string, ready: boolean, enabled: boolean, mode: Mode, cursor: bigint): InfrastructureState {
  const [state, setState] = useState<InfrastructureState & { scopeKey: string }>({ snapshot: null, error: null, scopeKey: "" });
  const at = useDebounced(infraAt(cursor), 300);
  const cursorKey = mode === "live" ? 0n : at;
  useEffect(() => {
    if (!ready || !enabled) return;
    let cancelled = false;
    const accept = (snapshot: InfrastructureSnapshot) => {
      if (!cancelled) setState({ snapshot, error: null, scopeKey });
    };
    if (mode === "live") {
      const unsubscribe = actions.onInfrastructure(accept);
      void actions.loadInfrastructure(nowNs(), true).then((snapshot) => { if (snapshot) accept(snapshot); }).catch((e: unknown) => {
        if (!cancelled) setState((s) => ({ snapshot: s.snapshot, error: e instanceof Error ? e.message : String(e), scopeKey }));
      });
      return () => { cancelled = true; unsubscribe(); };
    }
    void actions.loadInfrastructure(cursorKey).then((snapshot) => { if (snapshot) accept(snapshot); }).catch((e: unknown) => {
      if (!cancelled) setState((s) => ({ snapshot: s.snapshot, error: e instanceof Error ? e.message : String(e), scopeKey }));
    });
    return () => { cancelled = true; };
  }, [actions, scopeKey, ready, enabled, mode, cursorKey]);
  return state.scopeKey === scopeKey && ready && enabled ? state : { snapshot: null, error: null };
}
