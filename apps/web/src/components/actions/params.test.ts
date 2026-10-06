import { describe, expect, it } from "vitest";
import { initialValue, paramValues } from "./params";

const replicas = { name: "replicas", description: "Replicas", min: 1, max: 3, integer: true, boolean: false };
const enabled = { name: "enabled", description: "Flag on (1) or off (0)", min: 0, max: 1, integer: true, boolean: true };
const rate = { name: "error_rate", description: "Share", min: 0, max: 1, integer: false, boolean: false };

describe("action parameters", () => {
  it("starts switches off and numbers at their minimum", () => {
    expect(initialValue(enabled)).toBe("0");
    expect(initialValue(replicas)).toBe("1");
    expect(paramValues([replicas, enabled, rate], {})).toEqual({ replicas: 1, enabled: 0, error_rate: 0 });
  });

  it("sends switches as exactly 0 or 1", () => {
    expect(paramValues([enabled], { enabled: "1" })).toEqual({ enabled: 1 });
    expect(paramValues([enabled], { enabled: "0" })).toEqual({ enabled: 0 });
    expect(paramValues([enabled], { enabled: "true" })).toEqual({ enabled: 0 });
  });

  it("passes numbers through for the API to validate", () => {
    expect(paramValues([replicas, rate], { replicas: "3", error_rate: "0.25" })).toEqual({ replicas: 3, error_rate: 0.25 });
    expect(paramValues([replicas], { replicas: "9" })).toEqual({ replicas: 9 });
  });

  it("sends only the parameters of the chosen action", () => {
    expect(Object.keys(paramValues([enabled], { enabled: "1", replicas: "2", cmd: "id" }))).toEqual(["enabled"]);
    expect(paramValues([], { replicas: "2" })).toEqual({});
  });
});
