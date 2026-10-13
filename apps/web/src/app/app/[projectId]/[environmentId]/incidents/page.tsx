import { IncidentsPage } from "@/components/incidents/IncidentsPage";

export default async function Incidents({ params }: { params: Promise<{ projectId: string; environmentId: string }> }) {
  const { projectId, environmentId } = await params;
  return <IncidentsPage projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} />;
}
