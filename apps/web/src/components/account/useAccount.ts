"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api, unwrap, type Schemas, type SessionResponse } from "@/lib/api";

/** PATCH /account and put the returned user into the cached session. */
export function useUpdateAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (body: Schemas["AccountUpdateRequest"]) => unwrap(await api.PATCH("/api/v1/account", { body })),
    onSuccess: (user) => {
      qc.setQueryData<SessionResponse>(["session"], (s) => (s ? { ...s, user } : s));
    },
  });
}
