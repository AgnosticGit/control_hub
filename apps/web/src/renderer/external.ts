// How systems outside the project (third-party APIs it calls, uninstrumented callers such as a
// browser frontend) look on the map and in the inspector. Pure, no Pixi, unit-tested.

import type { EntityKindDto } from "@karbird/web-engine";
import { build, dashes } from "./edge-path";

/** Light and neutral: no service uses it, and it never reads as a health status. */
export const EXTERNAL_COLOR = 0xe2e8f0;

export function isExternalKind(kind: EntityKindDto | undefined): boolean {
  return kind === "external_dependency" || kind === "external_client";
}

export function externalLabel(kind: EntityKindDto | undefined): string | null {
  if (kind === "external_dependency") return "External service";
  if (kind === "external_client") return "External client";
  return null;
}

/** What the inspector says about where the node's data comes from. */
export function externalExplanation(kind: EntityKindDto | undefined): string | null {
  if (kind === "external_dependency") {
    return "Not part of this project: a third-party system called over HTTP. It sends no telemetry here; requests and answers are measured by the calling service's client spans.";
  }
  if (kind === "external_client") {
    return "Not part of this project: requests that arrived without an instrumented caller (browser, frontend, curl, partner system). Measured by the receiving service's server spans.";
  }
  return null;
}

export function nodeSubLabel(external: boolean, active: number, requests: number): string {
  const base = active > 0 ? `${active} active · ${requests} req/30s` : `${requests} req/30s`;
  return external ? `external · ${base}` : base;
}

/** Dashes (flat x, y pairs) around a rectangle, for the border of an external card. */
export function dashedRect(left: number, top: number, w: number, h: number, on = 7, off = 5): number[][] {
  const r = left + w;
  const b = top + h;
  return dashes(build([left, top, r, top, r, b, left, b, left, top]), on, off);
}
