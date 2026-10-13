export interface EdgeCardPoint {
  node: number;
  x: number;
  y: number;
}

export interface EdgeCardTarget {
  point: EdgeCardPoint;
  isPrimary: boolean;
}

export interface EdgeCardGroup {
  source: EdgeCardPoint;
  targets: EdgeCardTarget[];
  isPrimary: boolean;
}

/** Group destination cards under each source card so replicas can share one incoming trunk. */
export function edgeCardGroups(source: EdgeCardPoint, target: EdgeCardPoint, replicas: readonly EdgeCardPoint[]): EdgeCardGroup[] {
  const sources = [source, ...replicas.filter((card) => card.node === source.node)];
  const targets = [target, ...replicas.filter((card) => card.node === target.node)];
  return sources.map((from, sourceIndex) => ({
    source: from,
    targets: targets.map((to, targetIndex) => ({ point: to, isPrimary: sourceIndex === 0 && targetIndex === 0 })),
    isPrimary: sourceIndex === 0,
  }));
}
