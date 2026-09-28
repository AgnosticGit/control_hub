import type { MetadataRoute } from "next";

export default function sitemap(): MetadataRoute.Sitemap {
  const base = process.env.CONTROLHUB_SITE_URL ?? "http://localhost:3000";
  return ["", "/features", "/integrations", "/getting-started", "/docs"].map((path) => ({
    url: `${base}${path}`,
    changeFrequency: "weekly",
    priority: path === "" ? 1 : 0.7,
  }));
}
