/** Refresh a fixed selected-time snapshot, with one request in flight and no late commits. */
export function startInfrastructureRefresh<T>(load: () => Promise<T>, accept: (snapshot: T) => void, reject: (error: unknown) => void) {
  let cancelled = false;
  let busy = false;
  const refresh = async () => {
    if (cancelled || busy) return;
    busy = true;
    try {
      const snapshot = await load();
      if (!cancelled) accept(snapshot);
    } catch (error: unknown) {
      if (!cancelled) reject(error);
    } finally {
      busy = false;
    }
  };
  void refresh();
  const timer = setInterval(() => { void refresh(); }, 10_000);
  return () => { cancelled = true; clearInterval(timer); };
}
