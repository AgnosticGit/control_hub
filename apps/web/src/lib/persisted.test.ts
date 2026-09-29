import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readPersisted, subscribePersisted, writePersisted } from "./persisted";

class FakeStorage {
  readonly data = new Map<string, string>();
  broken = false;
  getItem(key: string): string | null {
    if (this.broken) throw new Error("SecurityError");
    return this.data.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    if (this.broken) throw new Error("QuotaExceededError");
    this.data.set(key, value);
  }
}

class FakeWindow extends EventTarget {
  readonly localStorage = new FakeStorage();
}

function storageEvent(key: string): Event {
  return Object.assign(new Event("storage"), { key });
}

let win: FakeWindow;
let n = 0;
const key = () => `test.persisted.${++n}`;

beforeEach(() => {
  win = new FakeWindow();
  vi.stubGlobal("window", win);
});

afterEach(() => vi.unstubAllGlobals());

describe("persisted preferences", () => {
  it("round-trips through localStorage", () => {
    const k = key();
    expect(readPersisted(k)).toBeNull();
    writePersisted(k, "metro");
    expect(win.localStorage.data.get(k)).toBe("metro");
    expect(readPersisted(k)).toBe("metro");
  });

  it("falls back to page memory when storage is unavailable", () => {
    const k = key();
    win.localStorage.broken = true;
    expect(() => writePersisted(k, "curved")).not.toThrow();
    expect(readPersisted(k)).toBe("curved");
  });

  it("notifies subscribers of local writes to their key only", () => {
    const a = key();
    const b = key();
    const fa = vi.fn();
    const off = subscribePersisted(a, fa);
    writePersisted(b, "x");
    expect(fa).not.toHaveBeenCalled();
    writePersisted(a, "y");
    expect(fa).toHaveBeenCalledTimes(1);
    off();
    writePersisted(a, "z");
    expect(fa).toHaveBeenCalledTimes(1);
  });

  it("follows writes from other tabs (storage events) until unsubscribed", () => {
    const k = key();
    const fn = vi.fn();
    const off = subscribePersisted(k, fn);
    win.dispatchEvent(storageEvent("other"));
    expect(fn).not.toHaveBeenCalled();
    win.dispatchEvent(storageEvent(k));
    expect(fn).toHaveBeenCalledTimes(1);
    off();
    win.dispatchEvent(storageEvent(k));
    expect(fn).toHaveBeenCalledTimes(1);
  });
});
