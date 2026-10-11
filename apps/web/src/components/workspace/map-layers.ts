export interface MapLayers {
  infrastructure: boolean;
  onlyProblems: boolean;
  hideInferred: boolean;
  tableView: boolean;
  serviceMetrics: boolean;
}

export const DEFAULT_MAP_LAYERS: MapLayers = {
  infrastructure: false,
  onlyProblems: false,
  hideInferred: false,
  tableView: false,
  serviceMetrics: false,
};

export function parseMapLayers(raw: string): MapLayers | null {
  try {
    const value: unknown = JSON.parse(raw);
    if (typeof value !== "object" || value === null) return null;
    const record = value as Record<string, unknown>;
    if (
      typeof record.infrastructure !== "boolean" ||
      typeof record.onlyProblems !== "boolean" ||
      typeof record.hideInferred !== "boolean" ||
      typeof record.tableView !== "boolean" ||
      (record.serviceMetrics !== undefined && typeof record.serviceMetrics !== "boolean")
    ) {
      return null;
    }
    return {
      infrastructure: record.infrastructure,
      onlyProblems: record.onlyProblems,
      hideInferred: record.hideInferred,
      tableView: record.tableView,
      serviceMetrics: record.serviceMetrics === true,
    };
  } catch {
    return null;
  }
}

export function needsInfrastructureSnapshot(layers: Pick<MapLayers, "infrastructure" | "serviceMetrics">): boolean {
  return layers.infrastructure || layers.serviceMetrics;
}
