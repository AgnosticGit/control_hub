// Public HTTPS address of the local dev stand for `just dev ngrok` (a tunnel to the web app on :3000).

export const DEFAULT_PUBLIC_URL = "https://slit-dandy-scallion.ngrok-free.dev";
export const DEFAULT_ALLOWED_ORIGINS = "http://localhost:3000,http://127.0.0.1:3000";

/** The public origin, or null when disabled. Only a bare https origin is accepted. */
export function publicUrl(env = process.env) {
  const raw = (env.CONTROLHUB_PUBLIC_URL ?? DEFAULT_PUBLIC_URL).trim();
  if (raw === "" || raw === "off") return null;
  let u;
  try {
    u = new URL(raw);
  } catch {
    throw new Error(`CONTROLHUB_PUBLIC_URL is not a URL: ${raw}`);
  }
  if (u.protocol !== "https:") throw new Error("CONTROLHUB_PUBLIC_URL must use https");
  if (u.username || u.password || u.pathname !== "/" || u.search || u.hash) {
    throw new Error("CONTROLHUB_PUBLIC_URL must be a bare origin like https://name.ngrok-free.dev");
  }
  return u.origin;
}

/** Origins the API accepts for cookie-authenticated mutations: the local ones plus the public one. */
export function allowedOrigins(pub, env = process.env) {
  const list = (env.CONTROLHUB_ALLOWED_ORIGINS ?? DEFAULT_ALLOWED_ORIGINS)
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  if (pub && !list.includes(pub)) list.push(pub);
  return list.join(",");
}
