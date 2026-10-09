import type { ReactNode } from "react";

const BASE_CLASS = "rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-error";

export function ErrorNotice({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <p role="alert" className={`${BASE_CLASS}${className ? ` ${className}` : ""}`}>{children}</p>;
}
