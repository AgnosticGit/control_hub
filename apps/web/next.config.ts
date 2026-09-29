import type { NextConfig } from "next";
import { baselineCsp } from "./src/lib/csp";

// Server-side only: where the Next.js server forwards /api/v1/* (same-origin
// for the browser, so session cookies stay SameSite=Strict and HttpOnly).
const apiUrl = process.env.CONTROLHUB_API_URL ?? "http://127.0.0.1:8080";
// Where /otlp/* is forwarded: the Rust ingest (OTLP/HTTP). Lets SDKs send telemetry through
// the same public address as the web app; tokens are checked by ingest, not here.
const ingestUrl = process.env.CONTROLHUB_INGEST_URL ?? "http://127.0.0.1:4318";
// Public tunnel hostname, set only by `just dev ngrok`; the dev server rejects other cross-origin hosts.
const publicHost = process.env.CONTROLHUB_DEV_PUBLIC_URL ? new URL(process.env.CONTROLHUB_DEV_PUBLIC_URL).hostname : null;

const securityHeaders = [
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Permissions-Policy", value: "camera=(), microphone=(), geolocation=()" },
  // `/app/*` additionally gets a strict nonce policy from proxy.ts; browsers enforce both.
  { key: "Content-Security-Policy", value: baselineCsp },
];

const nextConfig: NextConfig = {
  reactStrictMode: true,
  poweredByHeader: false,
  transpilePackages: ["@controlhub/api-client", "@controlhub/web-engine"],
  allowedDevOrigins: publicHost ? [publicHost] : [],
  async rewrites() {
    return [
      { source: "/api/v1/:path*", destination: `${apiUrl}/api/v1/:path*` },
      { source: "/otlp/v1/:signal", destination: `${ingestUrl}/v1/:signal` },
      { source: "/otlp/api/v1/events/batch", destination: `${ingestUrl}/api/v1/events/batch` },
    ];
  },
  async headers() {
    return [
      { source: "/:path*", headers: securityHeaders },
      {
        source: "/app/:path*",
        headers: [
          { key: "X-Robots-Tag", value: "noindex, nofollow" },
          { key: "Cache-Control", value: "no-store" },
        ],
      },
      {
        // Content-hashed engine binaries (scripts/build-wasm.mjs).
        source: "/wasm/:file*",
        headers: [
          { key: "Cache-Control", value: "public, max-age=31536000, immutable" },
          { key: "Content-Type", value: "application/wasm" },
        ],
      },
    ];
  },
};

export default nextConfig;
