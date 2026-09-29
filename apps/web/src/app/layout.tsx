import type { Metadata, Viewport } from "next";
import "./globals.css";

const siteUrl = process.env.CONTROLHUB_SITE_URL ?? "http://localhost:3000";

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title: {
    default: "ControlHub — observe, replay and safely operate your systems",
    template: "%s · ControlHub",
  },
  description:
    "ControlHub ingests OpenTelemetry traces and logs, builds a live service map and lets you replay system state over time.",
  openGraph: {
    type: "website",
    siteName: "ControlHub",
  },
};

export const viewport: Viewport = {
  themeColor: "#060a13",
  colorScheme: "dark",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="min-h-screen antialiased">{children}</body>
    </html>
  );
}
