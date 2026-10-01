export interface WorkspaceRoute {
  projectId: string;
  environmentId: string;
}

/** First segments under /app that are pages, not project ids. */
const APP_PAGES = new Set(["account", "admin", "invite", "login", "onboarding", "register"]);

function decode(segment: string): string | null {
  try {
    return decodeURIComponent(segment);
  } catch {
    return null;
  }
}

/** The project and environment of a `/app/{project}/{environment}[/...]` path. */
export function parseWorkspacePath(pathname: string): { route: WorkspaceRoute; overview: boolean } | null {
  const parts = pathname.split("/").filter(Boolean);
  if (parts[0] !== "app" || parts.length < 3) return null;
  const [p, e] = [parts[1] ?? "", parts[2] ?? ""];
  if (APP_PAGES.has(p)) return null;
  const projectId = decode(p);
  const environmentId = decode(e);
  if (!projectId || !environmentId) return null;
  return { route: { projectId, environmentId }, overview: parts.length === 3 };
}

export function sameRoute(a: WorkspaceRoute | null, b: WorkspaceRoute | null): boolean {
  return a === b || (a !== null && b !== null && a.projectId === b.projectId && a.environmentId === b.environmentId);
}

/**
 * The workspace kept alive after visiting `pathname`: opening an overview keeps that one; other
 * pages of the same environment and pages outside any project keep the current one; a page of
 * another environment drops it.
 */
export function keptWorkspace(kept: WorkspaceRoute | null, pathname: string): WorkspaceRoute | null {
  const parsed = parseWorkspacePath(pathname);
  if (!parsed) return kept;
  if (parsed.overview) return sameRoute(kept, parsed.route) ? kept : parsed.route;
  return sameRoute(kept, parsed.route) ? kept : null;
}
