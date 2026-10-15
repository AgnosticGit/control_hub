export const INCIDENT_CYCLE_MS = 10 * 60_000;
export const INCIDENT_BURST_MS = 3 * 60_000;
export const INCIDENT_STATE_INTERVAL_MS = 1000;

/** Three minutes of failed orders every ten minutes keeps the demo queue cycling. */
export function demoIncidentErrorRate(elapsedMs) {
  const phase = ((elapsedMs % INCIDENT_CYCLE_MS) + INCIDENT_CYCLE_MS) % INCIDENT_CYCLE_MS;
  return phase < INCIDENT_BURST_MS ? 0.5 : 0;
}

/** Switches the load generator's next scheduled request; no extra telemetry request is sent. */
export function startDemoIncidentCycle({ loadControlUrl, fetchImpl = fetch, log = () => {}, intervalMs = INCIDENT_STATE_INTERVAL_MS }) {
  const startedAt = Date.now();
  let enabled = true;
  let stopped = false;
  let sentState;
  const tick = async () => {
    if (stopped) return;
    const active = enabled && demoIncidentErrorRate(Date.now() - startedAt) > 0;
    if (sentState === active) return;
    try {
      const response = await fetchImpl(`${loadControlUrl.replace(/\/$/, "")}/__demo/incident-state`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ active }),
        signal: AbortSignal.timeout(2000),
      });
      if (!response.ok) throw new Error(`load generator incident control returned ${response.status}`);
      sentState = active;
    } catch (error) {
      if (!stopped) log(`incident state could not be sent: ${error instanceof Error ? error.message : String(error)}`);
    }
  };
  void tick();
  const timer = setInterval(() => void tick(), intervalMs);
  timer.unref?.();
  return {
    setEnabled(value) {
      enabled = value;
      void tick();
    },
    stop() {
      stopped = true;
      clearInterval(timer);
    },
  };
}
