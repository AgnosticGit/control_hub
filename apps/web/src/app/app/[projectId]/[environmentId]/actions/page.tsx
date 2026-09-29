import { ActionsPage } from "@/components/actions/ActionsPage";

export default async function Actions({ params }: { params: Promise<{ projectId: string; environmentId: string }> }) {
  const { projectId, environmentId } = await params;
  return <ActionsPage projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} />;
}
