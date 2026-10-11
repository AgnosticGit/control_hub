import { describe, expect, it } from "vitest";
import { cardGeometry, COMPACT_CARD_SIZE, EXPANDED_CARD_SIZE, routeBoxForCard } from "./map-card-geometry";

describe("service card geometry", () => {
  it("preserves compact dimensions and provides expanded dimensions for metrics", () => {
    expect(COMPACT_CARD_SIZE).toEqual({ width: 172, height: 56 });
    expect(EXPANDED_CARD_SIZE).toEqual({ width: 220, height: 104 });
    expect(cardGeometry(false)).toEqual(COMPACT_CARD_SIZE);
    expect(cardGeometry(true)).toEqual(EXPANDED_CARD_SIZE);
  });

  it("keeps route bounds and port spans aligned with the active card size", () => {
    expect(routeBoxForCard(COMPACT_CARD_SIZE)).toEqual({ hw: 90, hh: 32, portSpan: 44 });
    expect(routeBoxForCard(EXPANDED_CARD_SIZE)).toEqual({ hw: 114, hh: 56, portSpan: 92 });
  });
});
