import type { Schemas } from "@/lib/api";

export type LogSourceChoice = "auto" | "hot" | "archive";
export type LogSource = Schemas["LogSource"];
type LogPage = Schemas["LogListResponse"];
type Log = Schemas["LogDto"];

export const SOURCE_CHOICES: Array<{ value: LogSourceChoice; label: string; title: string }> = [
  { value: "auto", label: "Auto", title: "Recent logs from the hot store, older ones from the archive" },
  { value: "hot", label: "Hot", title: "Only the hot store (the last two weeks)" },
  { value: "archive", label: "Archive", title: "Only the long-term archive (closed hours)" },
];

/** Badge for where a page came from; `null` for the hot store, the usual case. */
export function sourceBadge(source: LogSource | undefined): { text: string; title: string } | null {
  if (source === "archive") return { text: "From archive", title: "These logs were read from the long-term archive." };
  if (source === "mixed") {
    return { text: "Hot + archive", title: "Newer logs from the hot store, older ones from the long-term archive." };
  }
  return null;
}

/** Timeline label when its log items (partly) came from the archive. */
export function timelineArchiveNote(source: LogSource | undefined): string | null {
  if (source === "archive") return "Logs from archive · no traces";
  if (source === "mixed") return "Older logs from archive · no traces there";
  return null;
}

/** Narrows an untyped value (e.g. from the engine Worker) to a log source. */
export function asLogSource(v: unknown): LogSource | undefined {
  return v === "hot" || v === "archive" || v === "mixed" ? v : undefined;
}

/** "Mixed" wins over a single store, so a list stitched from pages is labelled by all of them. */
export function combinedSource(pages: ReadonlyArray<Pick<LogPage, "source">>): LogSource | undefined {
  const seen = new Set(pages.map((p) => p.source));
  if (seen.size === 0) return undefined;
  if (seen.size > 1 || seen.has("mixed")) return "mixed";
  return pages[0]?.source;
}

/** Pages in order, with rows seen on an earlier page (live refetches can overlap) dropped. */
export function flattenPages(pages: ReadonlyArray<Pick<LogPage, "logs">>): Log[] {
  const seen = new Set<string>();
  const out: Log[] = [];
  for (const p of pages) {
    for (const l of p.logs) {
      if (seen.has(l.id)) continue;
      seen.add(l.id);
      out.push(l);
    }
  }
  return out;
}

export interface LogFilters {
  from: bigint;
  to: bigint;
  serviceId: string | null;
  minSeverity: number;
  q: string;
  source: LogSourceChoice;
  /** Lines of one kind (a `template_id` from a previous page). */
  templateId: string | null;
}

/** Query string of one page; only filters that are set are sent. */
export function logQuery(f: LogFilters, cursor: string | null, limit = 1000) {
  return {
    from: f.from.toString(),
    to: f.to.toString(),
    limit,
    ...(f.serviceId ? { service_id: f.serviceId } : {}),
    ...(f.minSeverity > 0 ? { min_severity: f.minSeverity } : {}),
    ...(f.q ? { q: f.q } : {}),
    ...(f.source !== "auto" ? { source: f.source } : {}),
    ...(f.templateId && f.templateId !== "0" ? { template_id: f.templateId } : {}),
    ...(cursor ? { cursor } : {}),
  };
}

/** Rows stored before templates existed have `"0"` and cannot be grouped. */
export function hasTemplate(l: Pick<Log, "template_id">): boolean {
  return /^[1-9]\d*$/.test(l.template_id);
}
