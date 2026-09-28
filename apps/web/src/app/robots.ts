import type { MetadataRoute } from "next";

export default function robots(): MetadataRoute.Robots {
  const base = process.env.CONTROLHUB_SITE_URL ?? "http://localhost:3000";
  return {
    rules: [{ userAgent: "*", allow: "/", disallow: ["/app", "/api"] }],
    sitemap: `${base}/sitemap.xml`,
  };
}
