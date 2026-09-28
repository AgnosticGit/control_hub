import { Workspace } from "@/components/workspace/Workspace";

export default async function WorkspacePage({
  params,
}: {
  params: Promise<{ projectId: string; environmentId: string }>;
}) {
  const { projectId, environmentId } = await params;
  return <Workspace projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} />;
}
