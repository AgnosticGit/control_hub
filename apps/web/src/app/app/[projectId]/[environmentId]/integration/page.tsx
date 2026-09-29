import { IntegrationPage } from "@/components/integration/IntegrationPage";

export default async function Integration({ params }: { params: Promise<{ projectId: string; environmentId: string }> }) {
  const { projectId, environmentId } = await params;
  return <IntegrationPage projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} />;
}
