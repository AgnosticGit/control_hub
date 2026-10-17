import { describe, expect, it } from "vitest";
import { installationStatus } from "./installation-status";

const now = Date.parse("2026-10-15T12:00:00Z");

describe("Control connection state", () => {
  it("shows a fresh, confirmed installation as connected", () => {
    expect(installationStatus({ revoked_at: null, confirmed_at: "2026-10-15T11:00:00Z", last_seen_at: "2026-10-15T11:59:59Z" }, now)).toBe("Connected");
  });

  it("shows lost heartbeats as disconnected even when previously confirmed", () => {
    expect(installationStatus({ revoked_at: null, confirmed_at: "2026-10-15T11:00:00Z", last_seen_at: "2026-10-15T11:59:30Z" }, now)).toBe("Disconnected");
  });

  it("keeps a fresh unconfirmed installation pending and a revoked one revoked", () => {
    expect(installationStatus({ revoked_at: null, confirmed_at: null, last_seen_at: "2026-10-15T11:59:59Z" }, now)).toBe("Awaiting confirmation");
    expect(installationStatus({ revoked_at: "2026-10-15T11:59:59Z", confirmed_at: null, last_seen_at: null }, now)).toBe("Revoked");
  });
});
