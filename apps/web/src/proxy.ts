import { NextResponse, type NextRequest } from "next/server";
import { strictCsp } from "@/lib/csp";

function nonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return btoa(String.fromCharCode(...bytes));
}

/** Pages under /app reachable without a session cookie. */
export function isPublicAppPath(pathname: string): boolean {
  return ["/app/login", "/app/register", "/app/invite"].some((p) => pathname === p || pathname.startsWith(`${p}/`));
}

// 1. Strict per-request CSP for the private app (Next.js applies the nonce from the request header).
// 2. Optimistic check only: redirects to login when there is no session cookie. Real
//    authentication and authorization happen in the Rust API on every call.
export function proxy(request: NextRequest) {
  const { pathname } = request.nextUrl;
  if (!isPublicAppPath(pathname) && !request.cookies.has("karbird_session")) {
    const url = request.nextUrl.clone();
    url.pathname = "/app/login";
    url.search = `?next=${encodeURIComponent(pathname)}`;
    return NextResponse.redirect(url);
  }
  const csp = strictCsp(nonce());
  const headers = new Headers(request.headers);
  headers.set("Content-Security-Policy", csp);
  const response = NextResponse.next({ request: { headers } });
  response.headers.set("Content-Security-Policy", csp);
  // Next.js replaces the `headers()` Cache-Control on rendered pages; private pages must never be stored.
  response.headers.set("Cache-Control", "no-store");
  return response;
}

export const config = {
  matcher: ["/app", "/app/:path*"],
};
