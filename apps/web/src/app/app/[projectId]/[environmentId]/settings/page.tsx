import { SettingsPage } from "@/components/settings/SettingsPage";

export default async function Settings({ params }: { params: Promise<{ projectId: string; environmentId: string }> }) {
  const { projectId, environmentId } = await params;
  return <SettingsPage projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} />;
}
