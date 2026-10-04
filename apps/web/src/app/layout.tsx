import type { Metadata, Viewport } from "next";
import { Hanken_Grotesk, JetBrains_Mono } from "next/font/google";
import "./globals.css";

const siteUrl = process.env.KARBIRD_SITE_URL ?? "http://localhost:3000";

const ui = Hanken_Grotesk({ subsets: ["latin"], variable: "--font-ui", display: "swap" });
const code = JetBrains_Mono({ subsets: ["latin"], variable: "--font-code", display: "swap" });

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title: {
    default: "karbird — observe, replay and safely operate your systems",
    template: "%s · karbird",
  },
  description:
    "karbird ingests OpenTelemetry traces and logs, builds a live service map and lets you replay system state over time.",
  openGraph: {
    type: "website",
    siteName: "karbird",
  },
};

export const viewport: Viewport = {
  themeColor: "#17181a",
  colorScheme: "dark",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${ui.variable} ${code.variable}`}>
      <body className="min-h-screen antialiased">{children}</body>
    </html>
  );
}
