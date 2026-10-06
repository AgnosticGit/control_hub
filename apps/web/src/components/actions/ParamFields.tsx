"use client";

import type { Schemas } from "@/lib/api";
import { initialValue } from "./params";

type Param = Schemas["ParamSpecDto"];

/** Inputs for an action's typed parameters: a checkbox for switches, a bounded number otherwise. */
export function ParamFields({
  params,
  values,
  onChange,
  compact = false,
}: {
  params: readonly Param[];
  values: Readonly<Record<string, string>>;
  onChange: (name: string, value: string) => void;
  compact?: boolean;
}) {
  return (
    <>
      {params.map((p) =>
        p.boolean ? (
          <label key={p.name} className="flex h-9 items-center gap-2" title={p.description}>
            <input
              type="checkbox"
              checked={(values[p.name] ?? initialValue(p)) === "1"}
              onChange={(e) => onChange(p.name, e.target.checked ? "1" : "0")}
              className="h-4 w-4"
            />
            <span className="label">{p.name}</span>
          </label>
        ) : (
          <label key={p.name} className="flex flex-col gap-1.5" title={p.description}>
            <span className="label">
              {p.name} ({p.min}–{p.max})
            </span>
            <input
              type="number"
              required
              min={p.min}
              max={p.max}
              step={p.integer ? 1 : 0.01}
              value={values[p.name] ?? initialValue(p)}
              onChange={(e) => onChange(p.name, e.target.value)}
              className={`input h-9 ${compact ? "w-24" : "w-28"}`}
            />
          </label>
        ),
      )}
    </>
  );
}
