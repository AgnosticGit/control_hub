/** The live inventory has one identity per scope; historical ranges keep exact identities. */
export function topologyCacheKey(url: string): string {
  const parsed = new URL(url, "http://worker.local");
  if (parsed.searchParams.get("live") === "true") {
    parsed.searchParams.delete("from");
    parsed.searchParams.delete("to");
  }
  parsed.searchParams.sort();
  return `${parsed.origin}${parsed.pathname}?${parsed.searchParams.toString()}`;
}

/** Empty maps are valid responses: an older request must not populate them afterward. */
export function applyTopologyResponse(sequence: number, latestSequence: number, apply: () => void): boolean {
  if (sequence !== latestSequence) return false;
  apply();
  return true;
}
