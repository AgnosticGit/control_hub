import { ENGINE_ABI_VERSION } from "@karbird/web-engine";

/** Versioned, best-effort history cache. Increment the IndexedDB version to invalidate all data. */
const TIMELINE_CACHE_SCHEMA_VERSION = 2;
export const TIMELINE_CACHE_DB_VERSION = TIMELINE_CACHE_SCHEMA_VERSION * 1_000 + ENGINE_ABI_VERSION;
export const TIMELINE_CACHE_LIMIT_BYTES = 1_073_741_824;
const DB_NAME = "karbird-timeline";
const CHUNKS = "chunks";
const RANGES = "ranges";
const META = "meta";

export interface TimelineRange { fromNs: string; toNs: string }
interface CachedRange extends TimelineRange { id: string; scopeKey: string; fromSort: string; touchedAt: number; bytes: number }
interface CachedChunk extends TimelineRange { id: string; scopeKey: string; rangeId: string; fromSort: string; sequence: number; itemsJson: string; bytes: number; touchedAt: number }
interface CacheMeta { id: "size"; bytes: number }
export interface CachedTimeline { chunks: Array<{ itemsJson: string }>; covered: TimelineRange[] }

export function missingTimelineRanges(from: bigint, to: bigint, coverage: readonly TimelineRange[]): TimelineRange[] {
  const ranges = coverage
    .map((range) => ({ from: BigInt(range.fromNs), to: BigInt(range.toNs) }))
    .filter((range) => range.from < to && range.to > from)
    .map((range) => ({ from: range.from < from ? from : range.from, to: range.to > to ? to : range.to }))
    .sort((a, b) => a.from < b.from ? -1 : a.from > b.from ? 1 : 0);
  const missing: TimelineRange[] = [];
  let cursor = from;
  for (const range of ranges) {
    if (range.from > cursor) missing.push({ fromNs: cursor.toString(), toNs: range.from.toString() });
    if (range.to > cursor) cursor = range.to;
  }
  if (cursor < to) missing.push({ fromNs: cursor.toString(), toNs: to.toString() });
  return missing;
}

export function oldestTimelineRange(ranges: readonly { id: string; touchedAt: number }[]): string | null {
  return ranges.reduce<{ id: string; touchedAt: number } | null>(
    (oldest, range) => !oldest || range.touchedAt < oldest.touchedAt ? range : oldest,
    null,
  )?.id ?? null;
}

function request<T>(value: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    value.onsuccess = () => resolve(value.result);
    value.onerror = () => reject(value.error ?? new Error("IndexedDB request failed"));
  });
}

function done(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = tx.onerror = () => reject(tx.error ?? new Error("IndexedDB transaction failed"));
  });
}

function openDb(): Promise<IDBDatabase> {
  if (typeof indexedDB === "undefined") return Promise.reject(new Error("IndexedDB is unavailable"));
  return new Promise((resolve, reject) => {
    const opening = indexedDB.open(DB_NAME, TIMELINE_CACHE_DB_VERSION);
    opening.onupgradeneeded = () => {
      const db = opening.result;
      for (const name of Array.from(db.objectStoreNames)) db.deleteObjectStore(name);
      const chunks = db.createObjectStore(CHUNKS, { keyPath: "id" });
      chunks.createIndex("scopeKey", "scopeKey");
      chunks.createIndex("rangeId", "rangeId");
      chunks.createIndex("scopeFrom", ["scopeKey", "fromSort"]);
      chunks.createIndex("touchedAt", "touchedAt");
      const ranges = db.createObjectStore(RANGES, { keyPath: "id" });
      ranges.createIndex("scopeKey", "scopeKey");
      ranges.createIndex("scopeFrom", ["scopeKey", "fromSort"]);
      db.createObjectStore(META, { keyPath: "id" });
    };
    opening.onsuccess = () => {
      const db = opening.result;
      db.onversionchange = () => { db.close(); cachePromise = null; };
      resolve(db);
    };
    opening.onerror = () => reject(opening.error ?? new Error("Could not open timeline cache"));
    opening.onblocked = () => reject(new Error("Timeline cache upgrade is blocked"));
  });
}

function rangeId(scopeKey: string, fromNs: string, toNs: string): string {
  return `${scopeKey}|${fromNs}|${toNs}`;
}

function sortNs(value: string): string { return value.padStart(20, "0"); }

export class TimelineCache {
  constructor(private readonly db: IDBDatabase, private readonly maxBytes = TIMELINE_CACHE_LIMIT_BYTES) {}

  async read(scopeKey: string, fromNs: string, toNs: string): Promise<CachedTimeline> {
    const tx = this.db.transaction([CHUNKS, RANGES], "readwrite");
    const chunksStore = tx.objectStore(CHUNKS);
    const rangesStore = tx.objectStore(RANGES);
    const from = BigInt(fromNs);
    const to = BigInt(toNs);
    const keyRange = IDBKeyRange.bound([scopeKey, "0"], [scopeKey, sortNs(toNs)], false, true);
    const [candidateChunks, candidateRanges] = await Promise.all([
      request(chunksStore.index("scopeFrom").getAll(keyRange)) as Promise<CachedChunk[]>,
      request(rangesStore.index("scopeFrom").getAll(keyRange)) as Promise<CachedRange[]>,
    ]);
    const matches = (range: TimelineRange) => BigInt(range.fromNs) < to && BigInt(range.toNs) > from;
    const chunks = candidateChunks.filter(matches).sort((a, b) => BigInt(a.fromNs) < BigInt(b.fromNs) ? -1 : BigInt(a.fromNs) > BigInt(b.fromNs) ? 1 : a.sequence - b.sequence);
    const covered = candidateRanges.filter(matches);
    const touchedAt = Date.now();
    for (const range of covered) rangesStore.put({ ...range, touchedAt });
    for (const chunk of chunks) chunksStore.put({ ...chunk, touchedAt });
    await done(tx);
    return { chunks: chunks.map(({ itemsJson }) => ({ itemsJson })), covered };
  }

  async putChunk(scopeKey: string, fromNs: string, toNs: string, sequence: number, itemsJson: string): Promise<boolean> {
    const bytes = new TextEncoder().encode(itemsJson).byteLength;
    if (bytes > this.maxBytes) return false;
    const id = `${rangeId(scopeKey, fromNs, toNs)}|${sequence}`;
    const value: CachedChunk = { id, scopeKey, rangeId: rangeId(scopeKey, fromNs, toNs), fromNs, toNs, fromSort: sortNs(fromNs), sequence, itemsJson, bytes, touchedAt: Date.now() };
    try {
      await this.storeChunk(value);
    } catch (error) {
      if (!isQuotaError(error)) return false;
      await this.evictOldest();
      try { await this.storeChunk(value); } catch { return false; }
    }
    try { await this.enforceLimit(); } catch { /* The cache is best effort. */ }
    const tx = this.db.transaction(CHUNKS, "readonly");
    const stored = await request(tx.objectStore(CHUNKS).get(id));
    await done(tx);
    return stored !== undefined;
  }

  async markComplete(scopeKey: string, fromNs: string, toNs: string): Promise<void> {
    const id = rangeId(scopeKey, fromNs, toNs);
    const tx = this.db.transaction(RANGES, "readwrite");
    tx.objectStore(RANGES).put({ id, scopeKey, fromNs, toNs, fromSort: sortNs(fromNs), touchedAt: Date.now(), bytes: 0 } satisfies CachedRange);
    await done(tx);
  }

  async clearScope(scopeKey: string): Promise<void> {
    const chunksTx = this.db.transaction(CHUNKS, "readonly");
    const rangesTx = this.db.transaction(RANGES, "readonly");
    const [removeChunks, removeRanges] = await Promise.all([
      request(chunksTx.objectStore(CHUNKS).index("scopeKey").getAll(scopeKey)) as Promise<CachedChunk[]>,
      request(rangesTx.objectStore(RANGES).index("scopeKey").getAll(scopeKey)) as Promise<CachedRange[]>,
    ]);
    await Promise.all([done(chunksTx), done(rangesTx)]);
    const tx = this.db.transaction([CHUNKS, RANGES, META], "readwrite");
    for (const chunk of removeChunks) tx.objectStore(CHUNKS).delete(chunk.id);
    for (const range of removeRanges) tx.objectStore(RANGES).delete(range.id);
    await this.adjustSize(tx, -removeChunks.reduce((sum, chunk) => sum + chunk.bytes, 0));
    await done(tx);
  }

  async clearAll(): Promise<void> {
    const tx = this.db.transaction([CHUNKS, RANGES, META], "readwrite");
    tx.objectStore(CHUNKS).clear();
    tx.objectStore(RANGES).clear();
    tx.objectStore(META).put({ id: "size", bytes: 0 } satisfies CacheMeta);
    await done(tx);
  }

  private async storeChunk(chunk: CachedChunk): Promise<void> {
    const tx = this.db.transaction([CHUNKS, META], "readwrite");
    const store = tx.objectStore(CHUNKS);
    const previous = await request(store.get(chunk.id)) as CachedChunk | undefined;
    store.put(chunk);
    await this.adjustSize(tx, chunk.bytes - (previous?.bytes ?? 0));
    await done(tx);
  }

  private async enforceLimit(): Promise<void> {
    while (true) {
      const metaTx = this.db.transaction(META, "readonly");
      const size = (await request(metaTx.objectStore(META).get("size")) as CacheMeta | undefined)?.bytes ?? 0;
      await done(metaTx);
      if (size <= this.maxBytes) return;
      if (!await this.evictOldest()) break;
    }
  }

  private async evictOldest(): Promise<boolean> {
    const ranges = await this.records(RANGES) as CachedRange[];
    const id = oldestTimelineRange(ranges.map(({ id, touchedAt }) => ({ id, touchedAt })));
    const range = ranges.find((item) => item.id === id);
    const victims = range ? await this.chunksForRange(range.id) : await this.oldestChunk();
    if (!range && victims.length === 0) return false;
    const tx = this.db.transaction([CHUNKS, RANGES, META], "readwrite");
    for (const chunk of victims) tx.objectStore(CHUNKS).delete(chunk.id);
    if (range) tx.objectStore(RANGES).delete(range.id);
    await this.adjustSize(tx, -victims.reduce((sum, chunk) => sum + chunk.bytes, 0));
    await done(tx);
    return true;
  }

  private async records(storeName: string): Promise<unknown[]> {
    const tx = this.db.transaction(storeName, "readonly");
    const result = await request(tx.objectStore(storeName).getAll());
    await done(tx);
    return result as unknown[];
  }

  private async chunksForRange(rangeIdValue: string): Promise<CachedChunk[]> {
    const tx = this.db.transaction(CHUNKS, "readonly");
    const result = await request(tx.objectStore(CHUNKS).index("rangeId").getAll(rangeIdValue)) as CachedChunk[];
    await done(tx);
    return result;
  }

  private async oldestChunk(): Promise<CachedChunk[]> {
    const tx = this.db.transaction(CHUNKS, "readonly");
    const value = await new Promise<CachedChunk | null>((resolve, reject) => {
      const cursor = tx.objectStore(CHUNKS).index("touchedAt").openCursor();
      cursor.onsuccess = () => resolve(cursor.result?.value as CachedChunk | undefined ?? null);
      cursor.onerror = () => reject(cursor.error ?? new Error("Could not find an old cached chunk"));
    });
    await done(tx);
    return value ? [value] : [];
  }

  private async adjustSize(tx: IDBTransaction, delta: number): Promise<void> {
    const store = tx.objectStore(META);
    const current = (await request(store.get("size")) as CacheMeta | undefined)?.bytes ?? 0;
    store.put({ id: "size", bytes: Math.max(0, current + delta) } satisfies CacheMeta);
  }
}

function isQuotaError(error: unknown): boolean {
  return typeof DOMException !== "undefined" && error instanceof DOMException && error.name === "QuotaExceededError";
}

let cachePromise: Promise<TimelineCache> | null = null;
export function openTimelineCache(): Promise<TimelineCache> {
  cachePromise ??= openDb().then((db) => new TimelineCache(db)).catch((error: unknown) => {
    cachePromise = null;
    throw error;
  });
  return cachePromise;
}
