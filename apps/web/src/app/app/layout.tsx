import type { Metadata } from "next";
import { WorkspaceHost } from "@/components/workspace/WorkspaceHost";
import { Providers } from "./providers";

// Rendered per request so Next.js can put the CSP nonce from proxy.ts on its scripts.
export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "App",
  robots: { index: false, follow: false },
};

export default function AppLayout({ children }: { children: React.ReactNode }) {
  return (
    <Providers>
      <WorkspaceHost>{children}</WorkspaceHost>
    </Providers>
  );
}
