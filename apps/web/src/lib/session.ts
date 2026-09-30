"use client";

import { useQuery } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { api, isUnauthenticated, unwrap } from "./api";

export function useSession() {
  const router = useRouter();
  const query = useQuery({
    queryKey: ["session"],
    queryFn: async () => unwrap(await api.GET("/api/v1/session")),
  });
  useEffect(() => {
    if (query.error && isUnauthenticated(query.error)) {
      const next = typeof window === "undefined" ? "/app" : window.location.pathname;
      router.replace(`/app/login?next=${encodeURIComponent(next)}`);
    }
  }, [query.error, router]);
  return query;
}

/** Signs out and reloads into the login page, so no cached data of the account survives and no
 *  query of the old page can redirect with its own `next`. */
export function useLogout() {
  return async () => {
    await api.POST("/api/v1/session/logout");
    window.location.replace("/app/login");
  };
}
