import { colorAt, cssColor } from "@/renderer/service-colors";

/** Stable line colour for a project, derived from its id. */
export function projectColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return cssColor(colorAt(h % 10));
}
