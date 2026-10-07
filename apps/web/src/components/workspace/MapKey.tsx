"use client";

import { useEffect, useId, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "@/components/ui/Icon";
import { CARD_BG, hex } from "@/renderer/map-colors";
import { MAP_KEY, type Swatch } from "./map-key";

const BG = hex(CARD_BG);

function SwatchIcon({ s }: { s: Swatch }) {
  return (
    <svg viewBox="0 0 32 16" className="h-6 w-12" aria-hidden>
      {s.kind === "dot" ? (
        <>
          {s.halo ? <circle cx="16" cy="8" r="7.5" fill={s.halo} opacity="0.35" /> : null}
          <circle
            cx="16"
            cy="8"
            r={s.size === "small" ? 2.6 : s.size === "large" ? 5.5 : 4}
            fill={s.color}
            opacity={s.faded ? 0.4 : 0.95}
            stroke={s.size === "small" ? "none" : "#ffffff"}
            strokeOpacity={s.faded ? 0.25 : 0.7}
            strokeWidth="1"
          />
        </>
      ) : s.kind === "ring" ? (
        <circle cx="16" cy="8" r="4" fill={BG} stroke={s.color} strokeWidth="2" />
      ) : s.kind === "line" ? (
        <>
          <line x1="2" y1="8" x2="26" y2="8" stroke={s.color} strokeWidth={s.thick ? 2.5 : 1.6} strokeDasharray={s.dashed ? "4 3" : undefined} />
          <path d="M25 4.5 30 8l-5 3.5z" fill={s.color} />
        </>
      ) : (
        <>
          {s.glow ? <rect x="3" y="1" width="26" height="14" rx="4" fill="none" stroke={s.glow} strokeWidth="3" opacity="0.3" /> : null}
          {s.outline ? <rect x="2" y="0.75" width="28" height="14.5" rx="4.5" fill="none" stroke={s.outline} strokeWidth="1.2" /> : null}
          <rect
            x="5"
            y="3"
            width="22"
            height="10"
            rx={s.dashed ? 1.5 : 3}
            fill={BG}
            stroke={s.color}
            strokeWidth="1.3"
            strokeOpacity={s.faded ? 0.4 : 0.9}
            strokeDasharray={s.dashed ? "3 2" : undefined}
          />
        </>
      )}
    </svg>
  );
}

interface Props {
  /** What the moving dots show right now (live delay, replay speed). */
  animation: string;
  /** Set when not every request in the window is drawn. */
  capped: string | null;
}

/** Info button for a full-screen map key, opened by click and closed by Esc, close or outside click. */
export function MapKey({ animation, capped }: Props) {
  const [open, setOpen] = useState(false);
  const id = useId();

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <div className="pointer-events-auto">
      <button
        type="button"
        className={`flex h-8 w-8 items-center justify-center rounded-lg border bg-surface/90 backdrop-blur transition-colors hover:text-text ${
          open ? "border-accent/60 text-accent" : "border-border text-muted"
        }`}
        aria-label="Show map key"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((isOpen) => !isOpen)}
      >
        <Icon name="info" className="h-4 w-4" />
      </button>
      {open
        ? createPortal(
            <div
              className="fixed inset-0 z-50 flex items-center justify-center bg-bg/75 p-6 backdrop-blur-sm"
              onClick={(e) => {
                if (e.target === e.currentTarget) setOpen(false);
              }}
            >
              <div
                id={id}
                role="dialog"
                aria-label="Map key"
                className="flex max-h-full w-full max-w-6xl flex-col rounded-2xl border border-border-strong bg-surface-2 p-6 shadow-2xl"
              >
                <header className="mb-5 flex items-start gap-4">
                  <div className="min-w-0 flex-1">
                    <h2 className="text-lg font-semibold text-text">Map key</h2>
                    <p className="mt-1 text-sm text-muted">{animation}</p>
                    {capped ? <p className="mt-1 text-sm text-warn">{capped}.</p> : null}
                  </div>
                  <button type="button" className="btn btn-sm" onClick={() => setOpen(false)} aria-label="Close map key">
                    <Icon name="x" className="h-4 w-4" />
                    Close
                  </button>
                </header>
                <div className="grid min-h-0 grid-cols-1 gap-x-8 gap-y-6 overflow-y-auto md:grid-cols-3">
                  {MAP_KEY.map((section) => (
                    <section key={section.title}>
                      <h3 className="mb-3 border-b border-border pb-2 text-xs font-semibold tracking-wider text-text uppercase">{section.title}</h3>
                      <ul className="space-y-3">
                        {section.items.map((item) => (
                          <li key={item.id} className="flex items-start gap-3">
                            <span className="flex shrink-0 items-center justify-center rounded-lg border border-border bg-bg/70 px-1.5 py-1">
                              <SwatchIcon s={item.swatch} />
                            </span>
                            <span className="min-w-0">
                              <span className="block text-sm font-semibold text-text">{item.label}</span>
                              <span className="block text-[13px] leading-snug text-muted">{item.meaning}</span>
                            </span>
                          </li>
                        ))}
                      </ul>
                    </section>
                  ))}
                </div>
              </div>
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
