export function workspaceSuffix(pathname: string): string {
  const parts = pathname.split("/").filter(Boolean);
  return parts.length > 3 ? `/${parts.slice(3).join("/")}` : "";
}

export function workspaceTarget(projectId: string, environmentId: string, pathname: string): string {
  return `/app/${encodeURIComponent(projectId)}/${encodeURIComponent(environmentId)}${workspaceSuffix(pathname)}`;
}

