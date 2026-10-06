import type { Schemas } from "@/lib/api";

type Param = Schemas["ParamSpecDto"];

/** Form value of a parameter the user has not touched: off for switches, the minimum otherwise. */
export function initialValue(p: Param): string {
  return p.boolean ? "0" : String(p.min);
}

/** Parameters as sent with a proposal. Switches are 0 or 1; the API validates every range. */
export function paramValues(params: readonly Param[], inputs: Readonly<Record<string, string>>): Record<string, number> {
  const values: Record<string, number> = {};
  for (const p of params) {
    const raw = inputs[p.name] ?? initialValue(p);
    values[p.name] = p.boolean ? (raw === "1" ? 1 : 0) : Number(raw);
  }
  return values;
}
