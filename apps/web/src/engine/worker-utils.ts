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

/** Reconnect delay after `attempt` consecutive failures: exponential, capped at 30 s, 50–100 % jitter. */
export function liveRetryDelayMs(attempt: number, random: number = Math.random()): number {
  const base = Math.min(30_000, 500 * 2 ** Math.min(attempt, 6));
  return Math.round(base * (0.5 + random / 2));
}

/** Seeks are coalesced: a pending seek is only replaced by one of the same or a newer generation. */
export function newerSeek<T extends { generation: number }>(pending: T | null, next: T): T {
  return !pending || next.generation >= pending.generation ? next : pending;
}
