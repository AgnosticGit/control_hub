"use client";

import { useSyncExternalStore } from "react";

const noSubscribe = () => () => {};

/** False during server render and hydration, true once React handles events on the client. */
export function useHydrated(): boolean {
  return useSyncExternalStore(
    noSubscribe,
    () => true,
    () => false,
  );
}
