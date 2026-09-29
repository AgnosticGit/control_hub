"use client";

import { useRef } from "react";

interface Props {
  /** "row": a horizontal bar that resizes the height of the panel below it; "column": a
   *  vertical bar that resizes the width of the panel to its right. */
  axis: "row" | "column";
  label: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}

/** Drag or arrow-key handle that resizes the panel after it (the panel grows towards the handle). */
export function Splitter({ axis, label, value, min, max, onChange }: Props) {
  const start = useRef<{ pos: number; value: number } | null>(null);
  const clamp = (v: number) => Math.round(Math.max(min, Math.min(max, v)));
  const pos = (e: React.PointerEvent) => (axis === "row" ? e.clientY : e.clientX);
  const row = axis === "row";
  return (
    <div
      role="separator"
      tabIndex={0}
      aria-label={label}
      aria-orientation={row ? "horizontal" : "vertical"}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      title={`${label} (drag or use the arrow keys)`}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { pos: pos(e), value };
      }}
      onPointerMove={(e) => {
        const s = start.current;
        if (s) onChange(clamp(s.value - (pos(e) - s.pos)));
      }}
      onPointerUp={(e) => {
        start.current = null;
        if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
      }}
      onKeyDown={(e) => {
        const grow = row ? "ArrowUp" : "ArrowLeft";
        const shrink = row ? "ArrowDown" : "ArrowRight";
        if (e.key !== grow && e.key !== shrink) return;
        e.preventDefault();
        onChange(clamp(value + (e.key === grow ? 24 : -24)));
      }}
      className={`group relative flex shrink-0 touch-none items-center justify-center outline-none ${
        row ? "h-3 cursor-row-resize" : "w-3 cursor-col-resize"
      }`}
    >
      <span
        aria-hidden
        className={`rounded-full bg-border-strong/0 transition-colors group-hover:bg-accent/70 group-focus-visible:bg-accent group-active:bg-accent ${
          row ? "h-1 w-16" : "h-16 w-1"
        }`}
      />
    </div>
  );
}
