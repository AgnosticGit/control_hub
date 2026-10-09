import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage, scopedPath, unwrap, type Schemas } from "@/lib/api";
import { OPEN_STATES } from "./controls";

export { errorMessage };

type Plan = Schemas["ActionPlanDto"];

export const ACTIONS_BASE = "/api/v1/projects/{project_id}/environments/{environment_id}/actions" as const;

/** Catalog, plans and the plan mutations of one environment; shared by the Actions page and the map. */
export function useActions(projectId: string, environmentId: string, enabled: boolean) {
  const qc = useQueryClient();
  const path = scopedPath(projectId, environmentId);
  const catalog = useQuery({
    queryKey: ["action-catalog", projectId, environmentId],
    enabled,
    queryFn: async () => unwrap(await api.GET(`${ACTIONS_BASE}/catalog`, { params: { path } })),
  });
  const plans = useQuery({
    queryKey: ["actions", projectId, environmentId],
    enabled,
    queryFn: async () => unwrap(await api.GET(ACTIONS_BASE, { params: { path } })),
    refetchInterval: (q) => ((q.state.data ?? []).some((p) => OPEN_STATES.has(p.state)) ? 2000 : 15_000),
  });
  const refresh = () => {
    void qc.invalidateQueries({ queryKey: ["actions", projectId, environmentId] });
    void qc.invalidateQueries({ queryKey: ["action", projectId, environmentId] });
  };
  const propose = useMutation({
    mutationFn: async (body: Schemas["ProposeActionRequest"]) => unwrap(await api.POST(ACTIONS_BASE, { params: { path }, body })),
    onSuccess: () => refresh(),
  });
  const decide = useMutation({
    mutationFn: async ({ plan, verb, comment }: { plan: Plan; verb: "approve" | "reject" | "cancel"; comment: string | null }) =>
      unwrap(await api.POST(`${ACTIONS_BASE}/{plan_id}/${verb}`, { params: { path: { ...path, plan_id: plan.id } }, body: { comment } })),
    onSuccess: () => refresh(),
  });
  return { catalog, plans, propose, decide };
}
