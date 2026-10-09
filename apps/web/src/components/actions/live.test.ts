import { describe, expect, it } from "vitest";
import { parseActionFrame, upsertActionPlan, type ActionPlan } from "./live";

const plan = (id: string, state = "running") => ({ id, state }) as ActionPlan;

describe("action live frames", () => {
  it("parses a server action frame and ignores unrelated or malformed frames", () => {
    expect(parseActionFrame(JSON.stringify({ type: "action", plan: plan("p1") }))).toEqual(plan("p1"));
    expect(parseActionFrame('{"type":"span"}')).toBeNull();
    expect(parseActionFrame('{"type":"action","plan":{"state":"running"}}')).toBeNull();
    expect(parseActionFrame("not json")).toBeNull();
  });

  it("upserts by plan ID without duplicating replayed messages", () => {
    const original = plan("p1", "proposed");
    const updated = plan("p1", "running");
    expect(upsertActionPlan([original], updated)).toEqual([updated]);
    expect(upsertActionPlan([original], original)).toEqual([original]);
    expect(upsertActionPlan(undefined, updated)).toEqual([updated]);
  });
});
