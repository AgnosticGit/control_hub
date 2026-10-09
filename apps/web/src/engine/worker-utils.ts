import type { InfrastructureSnapshot, LiveSignal } from "@karbird/web-engine";

// Pure helpers of the engine Worker (no WASM, no `self`), kept separate so they can be unit tested.

/** Refuse responses above this size instead of exhausting Worker memory. */
export const MAX_RESPONSE_BYTES = 64 * 1024 * 1024;

export interface TimelineTail {
  from_ns?: string;
  to_ns?: string;
  spans_truncated?: boolean;
  logs_truncated?: boolean;
  events_truncated?: boolean;
  logs_source?: string;
  completeness?: unknown;
}

/**
 * Reads the small metadata tail of a timeline response without parsing the
 * (potentially huge) `items` array in JS. Relies on the server emitting
 * `items` first (TimelineResponse field order); falls back to a full parse.
 */
export function timelineTail(text: string): TimelineTail {
  const marker = '],"from_ns":';
  const at = text.lastIndexOf(marker);
  if (at !== -1) {
    try {
      return JSON.parse(`{${text.slice(at + 2)}`) as TimelineTail;
    } catch {
      // fall through
    }
  }
  const full = JSON.parse(text) as TimelineTail;
  return full;
}

export function countItems(text: string): number {
  let n = 0;
  let i = text.indexOf('"type":"');
  while (i !== -1) {
    n++;
    i = text.indexOf('"type":"', i + 8);
  }
  return n;
}

/** Server messages start with `{"type":"…","resume":"…"`; only that head is read in JS. */
export function parseLiveHead(text: string): { type: string; resume: string | undefined } | null {
  const head = /^\{"type":"(\w+)"(?:,"resume":"(\d+)")?/.exec(text.slice(0, 128));
  if (!head || head[1] === undefined) return null;
  return { type: head[1], resume: head[2] };
}

export function parseLiveSignals(text: string): LiveSignal[] {
  try {
    // The API writes `signals` after the compact timeline items. Parse only this tail so the
    // Worker does not build a second JS object graph for the data WASM already consumed.
    const marker = ',"signals":';
    const at = text.lastIndexOf(marker);
    if (at < 0) return [];
    const resume = parseLiveHead(text)?.resume;
    if (!resume) return [];
    const value = JSON.parse(`{"signals":${text.slice(at + marker.length)}`) as { signals?: unknown };
    if (!Array.isArray(value.signals)) return [];
    return value.signals.filter((signal): signal is LiveSignal => {
      if (!signal || typeof signal !== "object") return false;
      const item = signal as { kind?: unknown; record?: unknown };
      return (item.kind === "span" || item.kind === "log" || item.kind === "event")
        && !!item.record && typeof item.record === "object" && !Array.isArray(item.record);
    }).map((signal) => {
      const storageCursor = (signal as unknown as Record<string, unknown>).storage_cursor;
      return { ...signal, resume, storageCursor: typeof storageCursor === "string" ? storageCursor : resume };
    });
  } catch {
    return [];
  }
}

export interface ParsedLiveRecord {
  kind: LiveSignal["kind"];
  resume: string;
  storageCursor: string;
  record: Record<string, unknown>;
  item: Record<string, unknown>;
}

export function parseLiveAction(text: string): { plan: Record<string, unknown>; snapshot: boolean } | null {
  try {
    const parsed = JSON.parse(text) as { type?: unknown; plan?: unknown; snapshot?: unknown };
    if (parsed.type !== "action" || !parsed.plan || typeof parsed.plan !== "object" || Array.isArray(parsed.plan)) return null;
    return { plan: parsed.plan as Record<string, unknown>, snapshot: parsed.snapshot === true };
  } catch {
    return null;
  }
}

/** Parse exactly one telemetry frame; individual signal messages stay small and bounded. */
export function parseLiveRecord(text: string): ParsedLiveRecord | null {
  try {
    const value = JSON.parse(text) as Record<string, unknown>;
    if (value.type !== "span" && value.type !== "log" && value.type !== "event") return null;
    if (typeof value.resume !== "string" || !/^\d+$/.test(value.resume)) return null;
    if (typeof value.storage_cursor !== "string" || !/^\d+$/.test(value.storage_cursor)) return null;
    if (!value.record || typeof value.record !== "object" || Array.isArray(value.record)) return null;
    if (!value.item || typeof value.item !== "object" || Array.isArray(value.item)) return null;
    return {
      kind: value.type,
      resume: value.resume,
      storageCursor: value.storage_cursor,
      record: value.record as Record<string, unknown>,
      item: value.item as Record<string, unknown>,
    };
  } catch {
    return null;
  }
}

/** Wrap one raw timeline item for the WASM engine without interpreting its fields. */
export function liveItemEnvelope(text: string): string | null {
  const parsed = parseLiveRecord(text);
  return parsed ? JSON.stringify({ items: [parsed.item] }) : null;
}

export function parseLiveInfrastructure(text: string): InfrastructureSnapshot | null {
  try {
    const value = JSON.parse(text) as { snapshot?: unknown };
    return value.snapshot && typeof value.snapshot === "object" && !Array.isArray(value.snapshot)
      ? value.snapshot as InfrastructureSnapshot
      : null;
  } catch {
    return null;
  }
}

/** Return the backend topology JSON body without materializing it on the Worker thread. */
export function liveTopologySnapshot(text: string): string | null {
  if (parseLiveHead(text)?.type !== "topology" || !text.endsWith("}")) return null;
  const marker = ',"snapshot":';
  const at = text.indexOf(marker);
  return at < 0 ? null : text.slice(at + marker.length, -1);
}

export function countLiveBatchItems(text: string): number {
  const startMarker = '"items":[';
  const start = text.indexOf(startMarker);
  if (start < 0) return 0;
  const end = text.indexOf(',"signals":', start + startMarker.length);
  const items = text.slice(start + startMarker.length, end < 0 ? text.length : end);
  return countItems(items);
}

/** Reconnect delay after `attempt` consecutive failures: exponential, capped at 30 s, 50–100 % jitter. */
export function liveRetryDelayMs(attempt: number, random: number = Math.random()): number {
  const base = Math.min(30_000, 500 * 2 ** Math.min(attempt, 6));
  return Math.round(base * (0.5 + random / 2));
}

/** Seeks are coalesced: a pending seek is only replaced by one of the same or a newer generation. */
export function newerSeek<T extends { generation: number }>(pending: T | null, next: T): T {
  return !pending || next.generation >= pending.generation ? next : pending;
}
