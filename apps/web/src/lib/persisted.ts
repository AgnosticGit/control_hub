import { useCallback, useSyncExternalStore } from "react";

// Small UI preferences remembered per browser. Values are validated on read, so a stale or
// hand-edited entry falls back to the default instead of breaking the page.

const listeners = new Map<string, Set<() => void>>();
const memory = new Map<string, string>();

function readRaw(key: string): string | null {
  try {
    return window.localStorage.getItem(key) ?? memory.get(key) ?? null;
  } catch {
    return memory.get(key) ?? null;
  }
}

export function writePersisted(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Storage can be unavailable (private mode); the value then lasts for this page only.
    memory.set(key, value);
  }
  for (const fn of listeners.get(key) ?? []) fn();
}

function subscribe(key: string, fn: () => void) {
  const set = listeners.get(key) ?? new Set();
  set.add(fn);
  listeners.set(key, set);
  const onStorage = (e: StorageEvent) => {
    if (e.key === key) fn();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    set.delete(fn);
    window.removeEventListener("storage", onStorage);
  };
}

/** A remembered string preference; `parse` returns null for invalid stored values. */
export function usePersisted<T>(key: string, fallback: T, parse: (raw: string) => T | null): T {
  const sub = useCallback((fn: () => void) => subscribe(key, fn), [key]);
  const raw = useSyncExternalStore(sub, () => readRaw(key), () => null);
  if (raw === null) return fallback;
  return parse(raw) ?? fallback;
}
