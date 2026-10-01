import { describe, expect, it } from "vitest";
import type { TimelineTracks } from "@controlhub/web-engine";
import type { ActionBar } from "./draw";
import { centeredOn, nearestItems, tracksMatch, viewIsEmpty } from "./neighbours";
import type { View } from "./scale";

const S = 1_000_000_000n;
const VIEW: View = { from: 100n * S, to: 110n * S };

function tracks(p: Partial<TimelineTracks> = {}): TimelineTracks {
  return {
    from_ns: VIEW.from.toString(),
    to_ns: VIEW.to.toString(),
    buckets: 3,
    requests: [0, 0, 0],
    errors: [0, 0, 0],
    events: [0, 0, 0],
    latency_p95_ms: [null, null, null],
    latency_samples: [0, 0, 0],
    services: [],
    clips: [],
    markers: [],
    markers_total: 0,
    prev_item_ns: null,
    next_item_ns: null,
    ...p,
  };
}

const bar = (from: bigint, to: bigint): ActionBar => ({ id: `${from}`, title: "Restart", target: "orders", state: "completed", from, to });

describe("timeline neighbours", () => {
  it("uses tracks only when they were computed for this exact view", () => {
    expect(tracksMatch(tracks(), VIEW)).toBe(true);
    expect(tracksMatch(tracks({ to_ns: (111n * S).toString() }), VIEW)).toBe(false);
    expect(tracksMatch(null, VIEW)).toBe(false);
  });

  it("calls a view empty only when nothing at all is drawn in it", () => {
    expect(viewIsEmpty(tracks(), VIEW, [])).toBe(true);
    expect(viewIsEmpty(tracks({ requests: [0, 1, 0] }), VIEW, [])).toBe(false);
    expect(viewIsEmpty(tracks({ errors: [1, 0, 0] }), VIEW, [])).toBe(false);
    expect(viewIsEmpty(tracks({ events: [0, 0, 2] }), VIEW, [])).toBe(false);
    const longSpan = { node: 1, name: "orders", counts: [0, 0, 0], spans_in_view: 1, clipped: true };
    expect(viewIsEmpty(tracks({ services: [longSpan] }), VIEW, []), "a span started earlier still runs through the view").toBe(false);
    expect(viewIsEmpty(tracks(), VIEW, [bar(90n * S, 120n * S)]), "an action running through the view").toBe(false);
    expect(viewIsEmpty(tracks(), VIEW, [bar(10n * S, 20n * S)])).toBe(true);
  });

  it("takes the nearest item on each side from the engine and from actions", () => {
    const t = tracks({ prev_item_ns: (80n * S).toString(), next_item_ns: (150n * S).toString() });
    expect(nearestItems(t, VIEW, [])).toEqual({ prev: 80n * S, next: 150n * S });
    expect(nearestItems(t, VIEW, [bar(85n * S, 95n * S), bar(130n * S, 140n * S)])).toEqual({ prev: 95n * S, next: 130n * S });
    expect(nearestItems(t, VIEW, [bar(10n * S, 20n * S), bar(200n * S, 210n * S)])).toEqual({ prev: 80n * S, next: 150n * S });
    expect(nearestItems(tracks(), VIEW, [])).toEqual({ prev: null, next: null });
    expect(nearestItems(tracks(), VIEW, [bar(130n * S, 140n * S)])).toEqual({ prev: null, next: 130n * S });
  });

  it("ignores malformed engine times instead of throwing", () => {
    expect(nearestItems(tracks({ prev_item_ns: "garbage", next_item_ns: "" }), VIEW, [])).toEqual({ prev: null, next: null });
  });

  it("centres the view on an item without changing the zoom, inside the bounds", () => {
    const bounds: View = { from: 0n, to: 1000n * S };
    expect(centeredOn(VIEW, 500n * S, bounds)).toEqual({ from: 495n * S, to: 505n * S });
    expect(centeredOn(VIEW, 2n * S, bounds)).toEqual({ from: 0n, to: 10n * S });
    expect(centeredOn(VIEW, 999n * S, bounds)).toEqual({ from: 990n * S, to: 1000n * S });
  });
});
