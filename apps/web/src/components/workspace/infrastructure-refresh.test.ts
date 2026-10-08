import { afterEach, expect, it, vi } from "vitest";
import { startInfrastructureRefresh } from "./infrastructure-refresh";

afterEach(() => { vi.clearAllTimers(); vi.useRealTimers(); });

it("refreshes a paused point when late metrics arrive", async () => {
  vi.useFakeTimers();
  const load = vi.fn().mockResolvedValueOnce({ cpu: null }).mockResolvedValue({ cpu: 0.95 });
  const accept = vi.fn();
  const stop = startInfrastructureRefresh(load, accept, vi.fn());
  await vi.advanceTimersByTimeAsync(0);
  expect(accept).toHaveBeenLastCalledWith({ cpu: null });
  await vi.advanceTimersByTimeAsync(10_000);
  expect(accept).toHaveBeenLastCalledWith({ cpu: 0.95 });
  expect(load).toHaveBeenCalledTimes(2);
  stop();
});

it("does not overlap slow requests or commit an obsolete selected point after cancellation", async () => {
  vi.useFakeTimers();
  let finish: (snapshot: number) => void = () => {};
  const load = vi.fn(() => new Promise<number>(resolve => { finish = resolve; }));
  const accept = vi.fn();
  const stop = startInfrastructureRefresh(load, accept, vi.fn());
  await vi.advanceTimersByTimeAsync(20_000);
  expect(load).toHaveBeenCalledTimes(1);
  stop();
  finish(1);
  await vi.advanceTimersByTimeAsync(20_000);
  expect(accept).not.toHaveBeenCalled();
  expect(load).toHaveBeenCalledTimes(1);
});

it("reports an outage and retries without publishing a failed snapshot", async () => {
  vi.useFakeTimers();
  const failure = new Error("unavailable");
  const load = vi.fn().mockRejectedValueOnce(failure).mockResolvedValue(2);
  const accept = vi.fn(), reject = vi.fn();
  const stop = startInfrastructureRefresh(load, accept, reject);
  await vi.advanceTimersByTimeAsync(0);
  expect(reject).toHaveBeenCalledWith(failure);
  expect(accept).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(accept).toHaveBeenCalledWith(2);
  stop();
});
