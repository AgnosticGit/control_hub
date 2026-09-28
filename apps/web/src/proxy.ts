import { NextResponse, type NextRequest } from "next/server";

// Optimistic check only: redirects to login when there is no session cookie.
// Real authentication and authorization happen in the Rust API on every call.
export function proxy(request: NextRequest) {
  if (!request.cookies.has("controlhub_session")) {
    const url = request.nextUrl.clone();
    url.pathname = "/app/login";
    url.search = `?next=${encodeURIComponent(request.nextUrl.pathname)}`;
    return NextResponse.redirect(url);
  }
  return NextResponse.next();
}

export const config = {
  matcher: ["/app", "/app/((?!login).*)"],
};
