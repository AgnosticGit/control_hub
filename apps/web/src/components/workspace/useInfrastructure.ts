"use client";

import { useEffect, useState } from "react";
import type { InfrastructureSnapshot } from "@karbird/web-engine";
import { infraAt } from "./infrastructure";
import { useDebounced } from "./useDebounced";
import type { WorkspaceActions } from "./useWorkspaceEngine";
import { startInfrastructureRefresh } from "./infrastructure-refresh";

export interface InfrastructureState {
  snapshot: InfrastructureSnapshot | null;
  error: string | null;
}

/** Loads machines and instances at the selected time while the infrastructure layer is on. */
export function useInfrastructure(actions: WorkspaceActions, ready: boolean, enabled: boolean, cursor: bigint): InfrastructureState {
  const [state, setState] = useState<InfrastructureState>({ snapshot: null, error: null });
  const at = useDebounced(infraAt(cursor), 300);
  useEffect(() => {
    if (!ready || !enabled) return;
    // A paused cursor can acquire late metrics and new placement just like a live view.
    // Live cursor changes restart this effect, so this does not add a second live poll.
    return startInfrastructureRefresh(
      () => actions.loadInfrastructure(at),
      (snapshot) => setState({ snapshot, error: null }),
      (e: unknown) => setState((s) => ({ snapshot: s.snapshot, error: e instanceof Error ? e.message : String(e) })),
    );
  }, [actions, ready, enabled, at]);
  return state;
}
