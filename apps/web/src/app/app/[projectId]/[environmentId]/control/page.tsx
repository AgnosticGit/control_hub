import { ControlPage } from "@/components/control/ControlPage";

export default async function Page({ params }: { params: Promise<{ projectId: string; environmentId: string }> }) {
  const { projectId, environmentId } = await params;
  return <ControlPage projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} />;
}
