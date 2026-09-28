import type { Metadata } from "next";
import { Providers } from "./providers";

export const metadata: Metadata = {
  title: "App",
  robots: { index: false, follow: false },
};

export default function AppLayout({ children }: { children: React.ReactNode }) {
  return <Providers>{children}</Providers>;
}
