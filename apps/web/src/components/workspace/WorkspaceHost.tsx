"use client";

import { usePathname } from "next/navigation";
import { useState, type ReactNode } from "react";
import { Workspace } from "./Workspace";
import { keptWorkspace, parseWorkspacePath, sameRoute, type WorkspaceRoute } from "./workspace-route";

/**
 * Keeps the last opened overview (engine, loaded data, map camera, selections) alive while the
 * user visits other pages, so coming back shows it at once instead of loading it again.
 */
export function WorkspaceHost({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const [kept, setKept] = useState<WorkspaceRoute | null>(null);
  const next = keptWorkspace(kept, pathname);
  if (next !== kept) setKept(next);
  const here = parseWorkspacePath(pathname);
  const visible = next !== null && here !== null && here.overview && sameRoute(here.route, next);
  return (
    <>
      {next ? <Workspace key={`${next.projectId}/${next.environmentId}`} projectId={next.projectId} environmentId={next.environmentId} visible={visible} /> : null}
      {children}
    </>
  );
}
