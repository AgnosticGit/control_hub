// Inline stroke icons (24x24 grid). Decorative: always aria-hidden; buttons carry their own labels.

const PATHS = {
  map: ["M3 6l6-3 6 3 6-3v15l-6 3-6-3-6 3z", "M9 3v15", "M15 6v15"],
  bolt: ["M13 2 4 14h7l-1 8 9-12h-7z"],
  key: ["M15.5 7.5a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0z", "M12 11v11", "M12 17h3", "M12 20h2"],
  search: ["M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z", "m20 20-3.5-3.5"],
  play: ["M7 4v16l13-8z"],
  pause: ["M8 5v14", "M16 5v14"],
  skipBack: ["M19 20 9 12l10-8z", "M5 19V5"],
  skipForward: ["M5 4l10 8-10 8z", "M19 5v14"],
  rewind: ["M11 19 2 12l9-7z", "M22 19l-9-7 9-7z"],
  forward: ["M13 19l9-7-9-7z", "M2 19l9-7-9-7z"],
  server: ["M4 4h16v6H4z", "M4 14h16v6H4z", "M8 7h.01", "M8 17h.01"],
  database: ["M12 8c4.4 0 8-1.3 8-3s-3.6-3-8-3-8 1.3-8 3 3.6 3 8 3z", "M4 5v14c0 1.7 3.6 3 8 3s8-1.3 8-3V5", "M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3"],
  activity: ["M22 12h-4l-3 9L9 3l-3 9H2"],
  alert: ["M12 3 2 21h20z", "M12 10v4", "M12 17h.01"],
  clock: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z", "M12 6v6l4 2"],
  logout: ["M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4", "m16 17 5-5-5-5", "M21 12H9"],
  layers: ["m12 2 10 5-10 5L2 7z", "m2 17 10 5 10-5", "m2 12 10 5 10-5"],
  globe: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z", "M2 12h20", "M12 2a15 15 0 0 1 0 20", "M12 2a15 15 0 0 0 0 20"],
  gauge: ["M12 14l4-4", "M3.3 19a10 10 0 1 1 17.4 0"],
  info: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z", "M12 16v-4", "M12 8h.01"],
  list: ["M8 6h13", "M8 12h13", "M8 18h13", "M3 6h.01", "M3 12h.01", "M3 18h.01"],
  radio: ["M12 14a2 2 0 1 0 0-4 2 2 0 0 0 0 4z", "M16.2 7.8a6 6 0 0 1 0 8.4", "M7.8 16.2a6 6 0 0 1 0-8.4", "M19.1 4.9a10 10 0 0 1 0 14.2", "M4.9 19.1a10 10 0 0 1 0-14.2"],
  cpu: ["M6 6h12v12H6z", "M9 9h6v6H9z", "M9 2v4", "M15 2v4", "M9 18v4", "M15 18v4", "M2 9h4", "M2 15h4", "M18 9h4", "M18 15h4"],
  x: ["M18 6 6 18", "m6 6 12 12"],
  plus: ["M12 5v14", "M5 12h14"],
  minus: ["M5 12h14"],
  maximize: ["M8 3H5a2 2 0 0 0-2 2v3", "M21 8V5a2 2 0 0 0-2-2h-3", "M3 16v3a2 2 0 0 0 2 2h3", "M16 21h3a2 2 0 0 0 2-2v-3"],
  arrowLeft: ["m12 19-7-7 7-7", "M19 12H5"],
  arrowRight: ["M5 12h14", "m12 5 7 7-7 7"],
  check: ["M20 6 9 17l-5-5"],
  shield: ["M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"],
  swap: ["M16 3l4 4-4 4", "M20 7H4", "M8 21l-4-4 4-4", "M4 17h16"],
  crosshair: ["M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z", "M21 12h-4", "M7 12H3", "M12 7V3", "M12 21v-4"],
  repeat: ["m17 2 4 4-4 4", "M3 11V10a4 4 0 0 1 4-4h14", "m7 22-4-4 4-4", "M21 13v1a4 4 0 0 1-4 4H3"],
  file: ["M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z", "M14 2v6h6"],
  calendar: ["M4 5h16v16H4z", "M16 3v4", "M8 3v4", "M4 11h16"],
  users: ["M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2", "M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z", "M22 21v-2a4 4 0 0 0-3-3.9", "M16 3.1a4 4 0 0 1 0 7.8"],
  user: ["M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2", "M12 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z"],
  link: ["M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7", "M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"],
  home: ["m3 10 9-7 9 7v10a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z"],
  folder: ["M3 6a2 2 0 0 1 2-2h4l2 3h8a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"],
  rocket: ["M5 15c-1.5 1.3-2 5-2 5s3.7-.5 5-2", "M9 12a22 22 0 0 1 11-9c0 4-2 8-9 11z", "M9 12H5l2-4h5", "M12 15v4l4-2v-5"],
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, className = "h-4 w-4", strokeWidth = 2 }: { name: IconName; className?: string; strokeWidth?: number }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      {PATHS[name].map((d) => (
        <path key={d} d={d} />
      ))}
    </svg>
  );
}

/** ControlHub mark: a hexagon with a node inside. */
export function Logo({ className = "h-7 w-7" }: { className?: string }) {
  return (
    <svg aria-hidden viewBox="0 0 32 32" className={className}>
      <defs>
        <linearGradient id="ch-logo" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#60a5fa" />
          <stop offset="1" stopColor="#22d3ee" />
        </linearGradient>
      </defs>
      <path d="M16 2.5 28 9.25v13.5L16 29.5 4 22.75V9.25z" fill="none" stroke="url(#ch-logo)" strokeWidth="2.5" strokeLinejoin="round" />
      <circle cx="16" cy="16" r="4.5" fill="url(#ch-logo)" />
    </svg>
  );
}
