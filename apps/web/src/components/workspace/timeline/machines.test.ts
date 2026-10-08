import { describe, expect, it } from "vitest";
import {
  MAX_MACHINE_BUCKETS,
  MAX_MACHINE_SPAN,
  MACHINE_TIMELINE_BUCKETS,
  eventLabel,
  eventMarkers,
  formatShare,
  holdSamples,
  machineAt,
  machineGeometry,
  machineWindow,
  type ContainerEvent,
  type InfraTimeline,
} from "./machines";
import { futureShare, withFuture } from "./scale";

const S = 1_000_000_000n;
const FROM = 1_800_000_000n * S;

function event(over: Partial<ContainerEvent>): ContainerEvent {
  return {
    time_ns: (FROM + 30n * S).toString(),
    kind: "restarted",
    host_id: "vps-app",
    host_name: "vps-app",
    container_id: "c1",
    container_name: "orders",
    service_name: null,
    detail: "orders restarted on vps-app",
    ...over,
  };
}

const T: InfraTimeline = {
  from_ns: FROM.toString(),
  to_ns: (FROM + 100n * S).toString(),
  buckets: 10,
  bucket_ns: (10n * S).toString(),
  hosts: [
    { id: "vps-app", name: "vps-app", cpu: [0.1, 0.2, null, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1], memory: [0.5, 0.5, null, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5] },
    { id: "vps-worker", name: "vps-worker", cpu: Array(10).fill(0.05), memory: Array(10).fill(0.25) },
  ],
  events: [
    event({}),
    event({ host_id: "vps-worker", host_name: "vps-worker", kind: "out_of_memory", time_ns: (FROM + 95n * S).toString() }),
    event({ host_id: "gone", time_ns: (FROM + 50n * S).toString() }),
    event({ time_ns: "not a number" }),
  ],
  events_total: 4,
  truncated: false,
};

describe("machine timeline", () => {
  it("reads CPU and memory of the bucket holding a time", () => {
    expect(machineAt(T, 0, FROM + 35n * S)).toEqual({ cpu: 0.4, memory: 0.5 });
    expect(machineAt(T, 0, FROM + 25n * S)).toEqual({ cpu: null, memory: null });
    expect(machineAt(T, 1, FROM)).toEqual({ cpu: 0.05, memory: 0.25 });
    expect(machineAt(T, 0, FROM - 1n)).toEqual({ cpu: null, memory: null });
    expect(machineAt(T, 0, FROM + 100n * S)).toEqual({ cpu: null, memory: null });
    expect(machineAt(T, 5, FROM)).toEqual({ cpu: null, memory: null });
  });

  it("formats shares as whole percent", () => {
    expect(formatShare(0.426)).toBe("43%");
    expect(formatShare(1)).toBe("100%");
    expect(formatShare(null)).toBe("–");
  });

  it("places visible events on their machine's row and skips unknown machines or bad times", () => {
    const view = { from: FROM, to: FROM + 100n * S };
    const m = eventMarkers(T, view, 1000);
    expect(m.map((e) => [e.host, Math.round(e.x)])).toEqual([
      [0, 300],
      [1, 950],
    ]);
    // Zoomed into the first half: the out-of-memory event is out of view.
    expect(eventMarkers(T, { from: FROM, to: FROM + 50n * S }, 1000).map((e) => e.event.kind)).toEqual(["restarted"]);
  });

  it("buckets are placed by their own size, independent of the view", () => {
    const g = machineGeometry(T, { from: FROM - 100n * S, to: FROM + 100n * S }, 1000);
    expect(g.x0).toBe(500);
    expect(g.bucketPx).toBe(50);
  });

  it("asks for a window covering the view that does not move with every small pan", () => {
    const view = { from: FROM + 3n * S, to: FROM + 903n * S };
    const w = machineWindow(view);
    expect(w).not.toBeNull();
    if (!w) return;
    expect(w.from <= view.from && w.to >= view.to).toBe(true);
    expect(machineWindow({ from: view.from + S, to: view.to + S })).toEqual(w);
    expect(w.buckets).toBe(MACHINE_TIMELINE_BUCKETS);
  });

  it("uses a fixed bucket count so viewport size cannot change the request", () => {
    const view = { from: FROM + 3n * S, to: FROM + 903n * S };
    expect(machineWindow(view)?.buckets).toBe(MACHINE_TIMELINE_BUCKETS);
  });

  it("excludes the visual future stretch from the request window", () => {
    const data = { from: FROM, to: FROM + 900n * S };
    const narrow = withFuture(data, futureShare(480));
    const wide = withFuture(data, futureShare(1440));
    expect(machineWindow(narrow, data)).toEqual(machineWindow(wide, data));
  });

  it("keeps the window within what the API accepts", () => {
    const huge = machineWindow({ from: FROM - 30n * 24n * 3600n * S, to: FROM });
    expect(huge && huge.to - huge.from).toBe(MAX_MACHINE_SPAN);
    expect(huge?.buckets).toBeLessThanOrEqual(MAX_MACHINE_BUCKETS);
    expect(machineWindow({ from: FROM, to: FROM })).toBeNull();
  });

  it("holds a machine's value over empty buckets between exports, up to 90 s", () => {
    const t: InfraTimeline = {
      ...T,
      bucket_ns: (30n * S).toString(),
      hosts: [{ id: "h", name: "h", cpu: [null, 0.2, null, null, null, null, 0.7, null], memory: [0.5, null, null, null, null, null, null, null] }],
    };
    const held = holdSamples(t).hosts[0];
    expect(held?.cpu).toEqual([null, 0.2, 0.2, 0.2, 0.2, null, 0.7, 0.7]);
    expect(held?.memory).toEqual([0.5, 0.5, 0.5, 0.5, null, null, null, null]);
    // Buckets longer than the hold time stay as they are.
    const coarse = { ...t, bucket_ns: (120n * S).toString() };
    expect(holdSamples(coarse)).toBe(coarse);
  });

  it("names a marker by what happened, to which container and where", () => {
    expect(eventLabel(event({}))).toBe("Container restarted: orders on vps-app");
    expect(eventLabel(event({ kind: "out_of_memory" }))).toBe("Out of memory: orders on vps-app");
    expect(eventLabel(event({ kind: "redeployed" }))).toBe("Redeployed: orders on vps-app");
  });
});
