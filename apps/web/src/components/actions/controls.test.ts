import { describe, expect, it } from "vitest";
import type { Schemas } from "@/lib/api";
import { OPEN_STATES, controlsFor, plansFor, stateClass } from "./controls";

type Spec = Schemas["ActionSpecDto"];
type Plan = Schemas["ActionPlanDto"];

const spec = (action_type: string, subjects: Array<[string, string, string | null]>, allowed_here = true): Spec => ({
  action_type,
  title: action_type,
  description: "",
  risk: "medium",
  demo_only: true,
  targets: subjects.map(([t]) => t),
  subjects: subjects.map(([target, service, container]) => ({ target, service, container })),
  params: [],
  verification: "",
  allowed_here,
});

const CATALOG: Spec[] = [
  spec("demo.faults.clear", [
    ["gateway", "gateway", null],
    ["orders", "orders", null],
  ]),
  spec("demo.restart_database", [["demo-postgres", "postgresql: orders", "postgres"]]),
  spec("demo.container.restart", [
    ["gateway", "gateway", "gateway"],
    ["orders", "orders", "orders"],
    ["catalog-1", "catalog", "catalog-1"],
    ["catalog-2", "catalog", "catalog-2"],
  ]),
  spec("demo.replicas.scale", [["catalog", "catalog", null]]),
  spec("demo.feature_flag.set", [
    ["catalog-cache", "gateway", null],
    ["checkout-v2", "orders", null],
  ]),
];

const summary = (cs: ReturnType<typeof controlsFor>) => cs.map((c) => `${c.spec.action_type}:${c.targets.join(",")}`);

describe("controls for a map card", () => {
  it("offers every action aimed at the service, its flags and its containers", () => {
    expect(summary(controlsFor(CATALOG, { service: "catalog" }))).toEqual([
      "demo.container.restart:catalog-1,catalog-2",
      "demo.replicas.scale:catalog",
    ]);
    expect(summary(controlsFor(CATALOG, { service: "gateway" }))).toEqual([
      "demo.faults.clear:gateway",
      "demo.container.restart:gateway",
      "demo.feature_flag.set:catalog-cache",
    ]);
    expect(summary(controlsFor(CATALOG, { service: "postgresql: orders" }))).toEqual(["demo.restart_database:demo-postgres"]);
  });

  it("offers nothing for services without actions or names that only look alike", () => {
    expect(controlsFor(CATALOG, { service: "auth" })).toEqual([]);
    expect(controlsFor(CATALOG, { service: "Catalog" })).toEqual([]);
    expect(controlsFor(CATALOG, { service: "" })).toEqual([]);
  });

  it("hides actions policy does not allow in this environment", () => {
    const notHere = CATALOG.map((s) => ({ ...s, allowed_here: false }));
    expect(controlsFor(notHere, { service: "gateway" })).toEqual([]);
  });
});

describe("controls for a container", () => {
  it("offers only what is aimed at that container", () => {
    expect(summary(controlsFor(CATALOG, { container: "catalog-2" }))).toEqual(["demo.container.restart:catalog-2"]);
    expect(summary(controlsFor(CATALOG, { container: "postgres" }))).toEqual(["demo.restart_database:demo-postgres"]);
    expect(summary(controlsFor(CATALOG, { container: "gateway" }))).toEqual(["demo.container.restart:gateway"]);
    expect(controlsFor(CATALOG, { container: "payments" })).toEqual([]);
  });
});

describe("plans shown next to the controls", () => {
  const plan = (id: string, action_type: string, target: string, state = "succeeded") => ({ id, action_type, target, state }) as Plan;
  const plans = [
    plan("1", "demo.container.restart", "catalog-2", "running"),
    plan("2", "demo.faults.clear", "gateway"),
    plan("3", "demo.container.restart", "gateway"),
    plan("4", "demo.replicas.scale", "catalog"),
    plan("5", "demo.container.restart", "catalog-1"),
  ];

  it("lists only the controls' own action and target pairs, newest first", () => {
    const container = controlsFor(CATALOG, { container: "gateway" });
    expect(plansFor(plans, container).map((p) => p.id)).toEqual(["3"]);
    const catalog = controlsFor(CATALOG, { service: "catalog" });
    expect(plansFor(plans, catalog).map((p) => p.id)).toEqual(["1", "4", "5"]);
    expect(plansFor(plans, catalog, 2).map((p) => p.id)).toEqual(["1", "4"]);
    expect(plansFor(plans, [])).toEqual([]);
  });

  it("marks open, failed and succeeded plans differently", () => {
    for (const s of OPEN_STATES) expect(stateClass(s)).toBe("text-warn");
    expect(stateClass("verification_failed")).toBe("text-error");
    expect(stateClass("succeeded")).toBe("text-ok");
    expect(stateClass("rejected")).toBe("text-muted");
  });
});
