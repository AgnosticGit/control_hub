import type { EntityKindDto } from "@controlhub/web-engine";
import { describe, expect, it } from "vitest";
import { EXTERNAL_COLOR, dashedRect, externalExplanation, externalLabel, isExternalKind, nodeSubLabel } from "./external";
import { RESOURCE_COLOR, colorAt } from "./service-colors";

const ALL: EntityKindDto[] = [
  "service",
  "instance",
  "repository",
  "deployment",
  "database",
  "cache",
  "queue",
  "infrastructure_resource",
  "external_dependency",
  "external_client",
];

describe("external systems", () => {
  it("only external dependencies and external clients count as external", () => {
    expect(ALL.filter(isExternalKind)).toEqual(["external_dependency", "external_client"]);
    expect(isExternalKind(undefined)).toBe(false);
  });

  it("labels and explains both kinds, and nothing else", () => {
    expect(externalLabel("external_dependency")).toBe("External service");
    expect(externalLabel("external_client")).toBe("External client");
    expect(externalExplanation("external_dependency")).toMatch(/Not part of this project/);
    expect(externalExplanation("external_client")).toMatch(/browser, frontend/);
    for (const k of ALL.filter((k) => !isExternalKind(k))) {
      expect(externalLabel(k)).toBeNull();
      expect(externalExplanation(k)).toBeNull();
    }
  });

  it("marks the card subtitle as external", () => {
    expect(nodeSubLabel(true, 0, 12)).toBe("external · 12 req/30s");
    expect(nodeSubLabel(true, 2, 12)).toBe("external · 2 active · 12 req/30s");
    expect(nodeSubLabel(false, 0, 3)).toBe("3 req/30s");
  });

  it("uses a colour no service or resource uses", () => {
    const used = new Set([RESOURCE_COLOR, ...Array.from({ length: 64 }, (_, i) => colorAt(i))]);
    expect(used.has(EXTERNAL_COLOR)).toBe(false);
  });

  it("dashes the whole card border and stays on it", () => {
    const d = dashedRect(0, 0, 100, 40, 7, 5);
    const perimeter = 2 * (100 + 40);
    expect(d.length).toBe(Math.ceil(perimeter / 12));
    for (const dash of d) {
      for (let i = 0; i + 1 < dash.length; i += 2) {
        const [x, y] = [dash[i] ?? NaN, dash[i + 1] ?? NaN];
        const onEdge = x === 0 || x === 100 || y === 0 || y === 40;
        expect(onEdge && x >= 0 && x <= 100 && y >= 0 && y <= 40).toBe(true);
      }
    }
  });
});
