import { describe, expect, it } from "vitest";
import { EXPANDED_CARD_SIZE } from "./map-card-geometry";
import { SERVICE_CARD_LAYOUT } from "./service-card-layout";

describe("expanded service card layout", () => {
  it("keeps the icon and title in a header above the CPU and RAM rows", () => {
    const { icon, title, cpu, memory } = SERVICE_CARD_LAYOUT;
    const iconRight = icon.x + icon.width;
    const iconBottom = icon.y + icon.height;

    expect(title.x).toBeGreaterThanOrEqual(iconRight + 8);
    expect(title.x + title.width).toBeLessThan(EXPANDED_CARD_SIZE.width - 8);
    expect(cpu.label.y).toBeGreaterThanOrEqual(iconBottom + 1);
    expect(memory.label.y).toBeGreaterThan(cpu.label.y);
  });

  it("keeps all metric rows inside the expanded card", () => {
    const rows = [
      SERVICE_CARD_LAYOUT.cpu.label, SERVICE_CARD_LAYOUT.cpu.value,
      SERVICE_CARD_LAYOUT.memory.label, SERVICE_CARD_LAYOUT.memory.value,
      SERVICE_CARD_LAYOUT.network.receive, SERVICE_CARD_LAYOUT.network.transmit,
      SERVICE_CARD_LAYOUT.requests.incoming, SERVICE_CARD_LAYOUT.requests.outgoing,
    ];
    for (const row of rows) {
      expect(row.x).toBeGreaterThanOrEqual(0);
      expect(row.y).toBeGreaterThanOrEqual(0);
      expect(row.x).toBeLessThan(EXPANDED_CARD_SIZE.width);
      expect(row.y + SERVICE_CARD_LAYOUT.textHeight).toBeLessThanOrEqual(EXPANDED_CARD_SIZE.height);
    }
  });
});
