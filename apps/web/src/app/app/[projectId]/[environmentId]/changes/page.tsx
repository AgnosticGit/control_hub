import { ChangesPage } from "@/components/changes/ChangesPage";

export default async function Changes({
  params,
  searchParams,
}: {
  params: Promise<{ projectId: string; environmentId: string }>;
  searchParams: Promise<{ event?: string }>;
}) {
  const [{ projectId, environmentId }, query] = await Promise.all([params, searchParams]);
  return <ChangesPage projectId={decodeURIComponent(projectId)} environmentId={decodeURIComponent(environmentId)} initialEventId={query.event ?? null} />;
}
