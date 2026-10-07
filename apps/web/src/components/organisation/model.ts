import type { ProjectDto } from "@/lib/api";

export function projectsForOrganization(projects: ProjectDto[], organizationId: string) {
  return projects.filter((project) => project.organization_id === organizationId);
}

export function assignmentLabel(project: ProjectDto | undefined, roleName: string | undefined) {
  return project ? `${project.name} · ${roleName ?? "assigned"}` : "Unknown project";
}

