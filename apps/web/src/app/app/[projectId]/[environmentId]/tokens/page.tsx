import { TokensPage } from "@/components/tokens/TokensPage";

export default async function Tokens({ params }: { params: Promise<{ projectId: string; environmentId: string }> }) {
  const { projectId, environmentId } = await params;
  return <TokensPage projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} />;
}
