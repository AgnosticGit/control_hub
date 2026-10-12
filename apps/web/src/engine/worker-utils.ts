import type { EdgeChange, InfrastructureSnapshot, LiveSignal } from "@karbird/web-engine";

export type TimelineFrame =
  | { type: "chunk"; requestId: string; sequence: number; fromNs: string; toNs: string }
  | { type: "range_complete"; requestId: string; sequence: number; fromNs: string; toNs: string }
  | { type: "complete"; requestId: string; truncated: boolean; completeness: unknown; logsSource?: string }
  | { type: "error"; requestId: string; message: string };

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

/** Parse only the small protocol header; history item arrays remain raw for the WASM engine. */
export function parseTimelineFrame(text: string): TimelineFrame | null {
  const type = parseLiveHead(text)?.type;
  if (type === "timeline_chunk") {
    const marker = ',"items":[';
    const at = text.indexOf(marker);
    if (at < 0) return null;
    try {
      const head = JSON.parse(`${text.slice(0, at)}}`) as Record<string, unknown>;
      const range = timelineRange(head.request_id, head.from_ns, head.to_ns);
      if (!range || !Number.isSafeInteger(head.sequence) || (head.sequence as number) < 0 || !findJsonArrayEnd(text, at + marker.length - 1)) return null;
      return { type: "chunk", requestId: range.requestId, fromNs: range.fromNs, toNs: range.toNs, sequence: head.sequence as number };
    } catch {
      return null;
    }
  }
  try {
    const value = JSON.parse(text) as Record<string, unknown>;
    if (type === "timeline_range_complete") {
      const range = timelineRange(value.request_id, value.from_ns, value.to_ns);
      return range && Number.isSafeInteger(value.sequence) && (value.sequence as number) >= 0
        ? { type: "range_complete", ...range, sequence: value.sequence as number }
        : null;
    }
    if (type === "timeline_complete") {
      const requestId = value.request_id;
      if (typeof requestId !== "string" || requestId.length === 0 || requestId.length > 128 || typeof value.truncated !== "boolean") return null;
      return {
        type: "complete", requestId, truncated: value.truncated,
        completeness: value.completeness ?? null,
        ...(typeof value.logs_source === "string" ? { logsSource: value.logs_source } : {}),
      };
    }
    if (type === "timeline_error" && typeof value.request_id === "string" && value.request_id.length > 0
      && value.request_id.length <= 128 && typeof value.message === "string") {
      return { type: "error", requestId: value.request_id, message: value.message };
    }
    return null;
  } catch {
    return null;
  }
}

/** Wrap a history chunk as the raw envelope expected by `WebEngine.applyBatch`. */
export function timelineChunkEnvelope(text: string): string | null {
  const marker = ',"items":[';
  const at = text.indexOf(marker);
  if (at < 0 || parseTimelineFrame(text)?.type !== "chunk") return null;
  const start = at + marker.length - 1;
  const end = findJsonArrayEnd(text, start);
  return end === null ? null : `{"items":${text.slice(start, end + 1)}}`;
}

/** Extract the raw JSON item array for IndexedDB without building JS objects. */
export function timelineChunkItemsJson(text: string): string | null {
  const envelope = timelineChunkEnvelope(text);
  return envelope?.startsWith('{"items":[') && envelope.endsWith("}")
    ? envelope.slice('{"items":'.length, -1)
    : null;
}

function timelineRange(requestId: unknown, fromNs: unknown, toNs: unknown) {
  if (typeof requestId !== "string" || requestId.length === 0 || requestId.length > 128
    || typeof fromNs !== "string" || !/^\d+$/.test(fromNs)
    || typeof toNs !== "string" || !/^\d+$/.test(toNs)) return null;
  try {
    if (BigInt(fromNs) >= BigInt(toNs)) return null;
    return { requestId, fromNs, toNs };
  } catch {
    return null;
  }
}

/** Find the closing bracket while respecting quoted strings and escaped characters. */
function findJsonArrayEnd(text: string, start: number): number | null {
  if (text[start] !== "[") return null;
  let depth = 0;
  let quoted = false;
  let escaped = false;
  for (let i = start; i < text.length; i++) {
    const char = text[i];
    if (quoted) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') quoted = false;
      continue;
    }
    if (char === '"') quoted = true;
    else if (char === "[") depth++;
    else if (char === "]" && --depth === 0) return i;
  }
  return null;
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

export function parseLiveEdgeChange(text: string): { resume: string; change: EdgeChange } | null {
  try {
    const parsed = JSON.parse(text) as { type?: unknown; resume?: unknown; change?: unknown };
    if (parsed.type !== "edge_change" || typeof parsed.resume !== "string" || !/^\d+$/.test(parsed.resume)
      || !parsed.change || typeof parsed.change !== "object" || Array.isArray(parsed.change)) return null;
    const change = parsed.change as Record<string, unknown>;
    if (["id", "tenant_id", "project_id", "environment_id", "source", "target", "relation", "trace_id", "span_id", "time_ns", "storage_cursor_ns"]
      .some((key) => typeof change[key] !== "string")
      || !/^\d+$/.test(change.storage_cursor_ns as string)
      || (change.request_bytes !== null && typeof change.request_bytes !== "number")
      || (change.response_bytes !== null && typeof change.response_bytes !== "number")) return null;
    return { resume: parsed.resume, change: change as unknown as EdgeChange };
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
