"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
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

export function useLogout() {
  const router = useRouter();
  const qc = useQueryClient();
  return async () => {
    await api.POST("/api/v1/session/logout");
    qc.clear();
    router.replace("/app/login");
  };
}
