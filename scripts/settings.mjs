// .karbird/settings.env: the developer's own settings for the local stand. Created with
// defaults by `just bootstrap` and never overwritten; real environment variables win over it.
// The services read it themselves (before the generated services.env).
import { existsSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const SETTINGS_FILE = "settings.env";

export const SETTINGS_TEMPLATE = `# Local karbird settings. Yours to edit; \`just bootstrap\` creates it once and never overwrites it.
# Real environment variables win over this file. Never commit .karbird/.

# How many days ClickHouse keeps each signal (2..3650). After a change run \`just bootstrap\`:
# it applies the new value as the table TTL. Until then the API and the archiver refuse to start.
# Shortening that deletes stored data needs \`just bootstrap --shrink-retention\`.
KARBIRD_LOGS_RETENTION_DAYS=14
KARBIRD_SPANS_RETENTION_DAYS=14
KARBIRD_EVENTS_RETENTION_DAYS=30
# Machine and container metrics from the Karbird Collector (Infrastructure layer of the map).
KARBIRD_METRICS_RETENTION_DAYS=14
`;

/** Writes the template if the file is missing; returns true when it was created. */
export function ensureSettings(dir) {
  const path = join(dir, SETTINGS_FILE);
  if (existsSync(path)) return false;
  writeFileSync(path, SETTINGS_TEMPLATE, { mode: 0o600 });
  return true;
}

/** `just bootstrap [--shrink-retention]` → extra args for `karbird-api bootstrap`. */
export function parseBootstrapArgs(argv) {
  const out = [];
  for (const a of argv) {
    if (a === "--shrink-retention") out.push("--shrink-retention");
    else throw new Error(`unknown argument ${JSON.stringify(a)} (supported: --shrink-retention)`);
  }
  return [...new Set(out)];
}
