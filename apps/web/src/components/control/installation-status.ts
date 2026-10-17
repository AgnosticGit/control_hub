export function installationStatus(
  installation: { revoked_at?: string | null; confirmed_at?: string | null; last_seen_at?: string | null },
  nowMs: number,
): string {
  if (installation.revoked_at) return "Revoked";
  if (!installation.last_seen_at || nowMs - Date.parse(installation.last_seen_at) > 20_000) return "Disconnected";
  return installation.confirmed_at ? "Connected" : "Awaiting confirmation";
}
