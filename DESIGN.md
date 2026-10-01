---
name: ControlHub
description: Observability, replay and controlled change for distributed systems, drawn as a transit line map on matte graphite.
colors:
  graphite-ground: "#17181a"
  graphite-raised: "#1c1d20"
  surface: "#212326"
  surface-2: "#27292d"
  surface-3: "#2e3135"
  hairline: "#33363b"
  hairline-strong: "#43474d"
  muted: "#9a9fa6"
  text: "#eceef0"
  sign: "#eceef0"
  sign-ink: "#17181a"
  accent: "#3b82f6"
  accent-2: "#22d3ee"
  violet: "#a855f7"
  warn: "#f59e0b"
  error: "#ef4444"
  ok: "#22c55e"
  line-telemetry: "#38bdf8"
  line-control: "#f472b6"
  line-emerald: "#34d399"
  line-teal: "#2dd4bf"
typography:
  display:
    fontFamily: "Hanken Grotesk, Segoe UI Variable, Segoe UI, ui-sans-serif, system-ui, sans-serif"
    fontSize: "2.5rem (sm 3.25rem, lg 2.75rem)"
    fontWeight: 800
    lineHeight: 1.05
    letterSpacing: "-0.03em"
    fontFeature: "\"ss01\", \"cv11\""
  headline:
    fontFamily: "Hanken Grotesk, Segoe UI Variable, Segoe UI, ui-sans-serif, system-ui, sans-serif"
    fontSize: "1.875rem"
    fontWeight: 700
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  title:
    fontFamily: "Hanken Grotesk, Segoe UI Variable, Segoe UI, ui-sans-serif, system-ui, sans-serif"
    fontSize: "2rem"
    fontWeight: 700
    lineHeight: 1.25
    letterSpacing: "-0.02em"
  body:
    fontFamily: "Hanken Grotesk, Segoe UI Variable, Segoe UI, ui-sans-serif, system-ui, sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.625
  body-lead:
    fontFamily: "Hanken Grotesk, Segoe UI Variable, Segoe UI, ui-sans-serif, system-ui, sans-serif"
    fontSize: "17px"
    fontWeight: 400
    lineHeight: 1.625
  label:
    fontFamily: "Hanken Grotesk, Segoe UI Variable, Segoe UI, ui-sans-serif, system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 500
    letterSpacing: "0.025em"
  mono:
    fontFamily: "JetBrains Mono, ui-monospace, SFMono-Regular, Cascadia Code, Menlo, Consolas, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.625
    fontFeature: "\"tnum\""
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
  full: "9999px"
spacing:
  hairline-gap: "2px"
  xs: "8px"
  sm: "12px"
  md: "16px"
  gutter: "24px"
  lg: "32px"
  xl: "48px"
  section: "80px"
components:
  button:
    backgroundColor: "{colors.surface-2}"
    textColor: "{colors.text}"
    rounded: "{rounded.lg}"
    padding: "0 12px"
    height: "32px"
  button-hover:
    backgroundColor: "{colors.surface-3}"
    textColor: "{colors.text}"
  button-primary:
    backgroundColor: "{colors.sign}"
    textColor: "{colors.sign-ink}"
    rounded: "{rounded.lg}"
    padding: "0 12px"
    height: "32px"
  button-primary-hover:
    backgroundColor: "#ffffff"
    textColor: "{colors.sign-ink}"
  button-hero:
    backgroundColor: "{colors.sign}"
    textColor: "{colors.sign-ink}"
    rounded: "{rounded.lg}"
    padding: "0 20px"
    height: "44px"
  button-sm:
    backgroundColor: "{colors.surface-2}"
    textColor: "{colors.text}"
    rounded: "{rounded.md}"
    padding: "0 10px"
    height: "28px"
  input:
    backgroundColor: "{colors.graphite-ground}"
    textColor: "{colors.text}"
    rounded: "{rounded.lg}"
    padding: "0 12px"
    height: "32px"
  badge:
    textColor: "{colors.muted}"
    rounded: "{rounded.md}"
    padding: "2px 6px"
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.lg}"
  nav-link:
    textColor: "{colors.muted}"
    rounded: "{rounded.md}"
    padding: "8px 12px"
  nav-link-active:
    backgroundColor: "{colors.surface-3}"
    textColor: "{colors.text}"
  environment-stop:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    rounded: "{rounded.md}"
    padding: "0 14px"
    height: "40px"
---

# Design System: ControlHub

## Overview

**Creative North Star: "The Metro Diagram"**

ControlHub draws a distributed system the way a city draws its metro: services are stations on coloured lines, requests are trains running between them, and time is a timetable rail you can scrub. The whole product sits on a matte graphite ground with hairline rules, so the only things that carry colour are the lines themselves and the health of what they connect. Everything else (chrome, cards, navigation, forms) stays neutral and quiet.

Density is professional-SaaS, not dashboard-crowded: landing sections breathe on wide 80px bands, the account area uses a narrow reading column, and the workspace keeps compact 32px controls. Depth comes from stepping graphite tones up, not from glow, gradient washes or lifted tiles. Primary actions read like wayfinding signs: a light sign fill with dark ink, one per region.

The line language is used everywhere, not only on the map: project identity is a short line swatch plus a station ring, the auth side panel is a three-line interchange, the onboarding checklist is a line of stops, and the architecture section is a telemetry line with a control line branching at the API interchange.

**Key Characteristics:**
- Matte graphite ground, five tonal steps, hairline borders; no gradients or glow.
- Hue only on service lines and status; chrome is neutral.
- Signage-style primary buttons: light fill, graphite ink.
- One grotesk (Hanken Grotesk) for UI and display, heavy and tight for headlines; JetBrains Mono for timestamps, IDs, metrics and commands.
- Stations are graphite-filled rings; solid strokes mean observed, dotted or dashed mean inferred or external.

## Colors

A neutral graphite scale carries the interface; a small set of saturated line hues and three status colours are the only voices.

### Primary
- **Signal White** (sign / text): the primary action fill (signage button) and all primary text. Hover lifts the sign to pure white.
- **Interchange Blue** (accent): interaction only. Focus rings, caret, text selection, range fill, selected segmented items, the `actions` marker on the demo rail. Pinned by the workspace timeline renderer.

### Secondary
- **Live Cyan** (accent-2): the second interaction hue in the workspace timeline (replay cursor, events band) and one stroke of the logo. Pinned by timeline tests.

### Tertiary
- **Telemetry Sky** (line-telemetry): the telemetry line on the landing interchange diagram and auth panel; also the observed-edge swatch.
- **Control Pink** (line-control): the control line (plan, policy, approval, executor, verified, audit).
- **Emerald Line** (line-emerald) and **Teal Line** (line-teal): additional service lines from the shared 12-hue service palette; teal is the inferred-edge swatch in the legend.
- **Violet** (violet): workspace timeline action/plan bands only.

Service lines draw from one shared 12-hue palette (sky, violet, emerald, pink, yellow, indigo, orange, teal, lime, fuchsia, pale blue, sand) in `renderer/service-colors.ts`; projects take a stable hue from it by id hash.

### Neutral
- **Graphite Ground** (graphite-ground): page background, station fill, input fill (at 70%), code blocks, sign-button ink.
- **Raised Graphite** (graphite-raised): alternating landing bands, the demo frame, auth side panel, account sidebar.
- **Surface / Surface 2 / Surface 3**: cards and environment stops; button rest; hover and active navigation.
- **Hairline / Hairline Strong**: every border and divider; strong for hover borders, dashed empty states, link underlines, avatar rings.
- **Muted Steel** (muted): secondary text, notes, nav at rest, timestamps on the rail.

### Status
- **Ok** (ok): success buttons and the Live indicator. On the landing map a healthy station ring is Signal White, not green.
- **Warn** (warn): degraded.
- **Error** (error): failing stations, failed trains, error banners, danger buttons.

### Named Rules
**The Line-Colour Rule.** Hue belongs to service lines and to status. Chrome (buttons, cards, navigation, headings) stays graphite and white. If a coloured element is neither a line, a status, nor an interaction cue, it is wrong.

**The Interaction-Blue Rule.** Interchange Blue marks where the user is acting (focus, selection, slider fill). It is never a decorative fill, a section background or a heading colour.

## Typography

**Display Font:** Hanken Grotesk (with Segoe UI Variable, system sans)
**Body Font:** Hanken Grotesk
**Label/Mono Font:** JetBrains Mono (with Cascadia Code, Menlo, Consolas)

**Character:** One neutral grotesk does every human-facing job, cut heavy with tight tracking for headlines so it reads like station signage. The mono is the machine voice: anything a system emitted (time, IDs, metrics, commands) is set in it. Stylistic sets `ss01` and `cv11` are on globally.

### Hierarchy
- **Display** (800, 2.5rem → 3.25rem at sm → 2.75rem at lg, line-height 1.05, -0.03em, balanced): the landing hero headline only.
- **Headline** (700, 1.875rem, -0.02em, balanced): landing section heads; may lead with a short line swatch (32×6px) in its line colour.
- **Title** (700, 2rem, leading-tight, -0.02em): page titles in the project picker and account area; auth forms use 1.75rem.
- **Body** (400, 16px, line-height 1.625): paragraphs in muted, capped at `max-w-md`/`max-w-lg` (~28–32rem). The hero lead is 17px. UI body is 14px (`text-sm`).
- **Label** (500, 11px, tracking 0.025em, uppercase): table heads and small form labels in the workspace only. Badges use the same cut at 10px semibold.
- **Mono** (400, 13px code blocks; 10–12px on rails and metrics, tabular numbers): timestamps, IDs, durations, error rates, CLI commands.

### Named Rules
**The One Grotesk Rule.** Display and UI share one family; hierarchy comes from weight (800/700/600) and tracking, never from a second display face.

**The Machine-Voice Rule.** If the system produced the value (a time, a duration, an ID, a command), set it in mono with tabular numbers. Human labels next to it stay in the grotesk.

## Layout

Public pages use a centred 1280px container (`max-w-7xl`) with 24px gutters. Landing sections are full-bleed bands separated by a hairline top border, 80px vertical padding (96px for the closing band), alternating graphite-ground and raised-graphite backgrounds. The hero and legend use a 5:7 asymmetric two-column grid at `lg`, collapsing to one column below; other bands split 1:1. The header is sticky, graphite at 90% with a light backdrop blur and a hairline bottom.

The auth screens split the viewport 1:1 at `lg`: form column left (max 24rem form), raised-graphite diagram panel right with a hairline left border; below `lg` the panel is hidden. The account area is a 248px raised sidebar plus a 896px (`max-w-4xl`) content column; on mobile the sidebar becomes a top bar with wrapped nav and icon buttons. The project picker uses the same 896px column with divided rows (name column 14rem, environments wrap).

Spacing rhythm is 4px-based: 8 / 12 / 16 / 24 / 32 / 48 / 80. Stacked headline → lead gaps are 16–24px; lead → actions 32–36px.

## Elevation & Depth

The system is flat and tonal. Depth is conveyed by stepping graphite (ground → raised → surface → surface-2 → surface-3) and by hairline borders. The only shadows are short, dark and functional: a two-layer ambient shadow under workspace cards to separate them from the canvas, and a small drop under slider thumbs so they read above the track. No coloured shadows, no glows, no gradient washes.

### Shadow Vocabulary
- **Card ambient** (`box-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.35), 0 8px 20px -14px rgb(0 0 0 / 0.7)`): workspace `.card` containers only.
- **Thumb drop** (`box-shadow: 0 1px 3px rgb(0 0 0 / 0.6)`): range and timeline slider thumbs.

### Named Rules
**The Matte Rule.** No gradients, no glow, no blur halos. A failing station may emit an expanding red ring while live (stroke only, 1.6s, off under reduced motion); that is a status signal, not decoration.

## Shapes

Gently rounded rectangles for chrome: 8px for buttons, inputs, cards and frames; 6px for small buttons, badges, nav items and environment stops; 4px for tiny timeline buttons. Everything that belongs to the line language is fully round: lines are 4–6px strokes with round caps and joins, project swatches are 24×6px pills, stations are circles.

Stations are graphite-filled rings: a regular stop has a 3px light ring, a terminus a 5px ring, an interchange a larger ring with a 4px stroke. External systems use a dashed ring. Observed edges are solid 6px; inferred edges are thinner (3px) and dotted (`2 9`). The logo is the same grammar: a cyan vertical line, a blue line with a 45° bend, and a ringed interchange where they cross.

## Components

### Buttons
Quiet graphite controls; the primary action is a sign.
- **Shape:** gently curved (8px; 6px at small size).
- **Default:** surface-2 fill, hairline border, text in Signal White, 32px tall, 12px horizontal padding, 12px medium type.
- **Primary (signage):** Signal White fill and border, graphite ink, semibold. Hero and closing CTAs grow to 44px with 20px padding and 14px type; auth submit is 40px full-width.
- **Hover / Focus:** default steps to surface-3 with a strong hairline; primary lifts to pure white. Focus is a 2px Interchange Blue outline at 2px offset. Disabled drops to 40% opacity.
- **Ok / Danger:** tinted (10% fill, 40% border) in ok or error; hover deepens to 15% / 70%.
- **Icon / Small:** 32px square icon button; small is 28px, 11px type, 6px radius.

### Chips
- **Badge:** 10px semibold uppercase with 0.025em tracking, 6px radius, 1px border in the status or hairline-strong colour, text in the same colour. Used for `DEMO` and status tags, never as headings.
- **Segmented control:** graphite-ground at 60% track, 6px items; selected item takes accent at 15% with accent text.

### Cards / Containers
- **Corner Style:** 8px.
- **Background:** surface; demo frame and code blocks use raised graphite or ground.
- **Shadow Strategy:** workspace cards carry the card ambient shadow; landing and account containers are border-only.
- **Border:** 1px hairline; headers are separated by a hairline bottom.
- **Internal Padding:** 16px (card header 16×10px), 20–24px for account panels and code blocks.

### Inputs / Fields
- **Style:** 1px hairline, graphite ground at 70%, 8px radius, 32px tall (40px on auth), 14px text, muted placeholder.
- **Focus:** border shifts to accent at 70% plus a 2px accent ring at 20%; caret is accent.
- **Error:** inline alert below the fields: error at 10% fill, 40% border, error text, 6px radius.
- **Switch:** native checkbox drawn as a 28×16 pill; checked fills accent with a white knob.

### Navigation
- **Public header:** logo mark + "ControlHub" in 17px bold; links 14px muted, 6px radius, hover to surface-2 and text; "Open app" is a 36px sign button at the end. Links scroll horizontally on mobile.
- **Account sidebar:** 14px items with 16px icons, 6px radius; active item is surface-3 + semibold text with `aria-current`; inactive muted with surface-2 hover.

### Line Swatch and Station (signature)
The reusable identity atom. A 24–40px round-capped pill in a line colour names a project, a section's line or a legend entry; a graphite-filled ring in the same colour marks a stop (an environment in the picker, a step in onboarding). Environment links are 40px surface pills with a 12px coloured station ring, name, optional `DEMO` badge and a muted arrow that nudges 2px right on hover.

### Map Demo (signature)
The landing page shows the product itself, not an illustration of it. `MapDemo` is the workspace's System Map and timeline, rebuilt in SVG and running on a synthetic system. It imports the workspace's colour sources (`renderer/map-colors.ts`, `renderer/service-colors.ts`, `nodeSubLabel`), so the two cannot drift apart.

The frame is raised graphite with a hairline title bar: "System Map" and `checkout · production` on the left, `Demo · synthetic data` in mono on the right. Each service is a 12px-radius card in its own service colour, with a status dot and a mono `N req/30s` line. When a service degrades or fails, its card takes the status colour. External systems get dashed 4px cards, and nodes known only by inference are dimmed. Edges are curves:
- solid blue when observed;
- dashed slate when inferred;
- amber or red when they touch a degraded or failing service.

Cyan dots are requests.

Below the map sit the workspace legend and a timeline with the workspace rows (Actions, Events, Errors, Requests, Latency p95). A cyan cursor marks the current time, and the whole track area is the scrub target. Wide screens lay the map out left to right; narrow screens stack it top to bottom.

## Do's and Don'ts

### Do:
- **Do** keep chrome on the graphite scale and put hue only on lines and status (The Line-Colour Rule).
- **Do** make the single primary action per region a Signal White sign with graphite ink; everything else is a graphite button or an underlined text link (hairline-strong underline, text-colour on hover).
- **Do** draw relationships as lines and stops: round-capped strokes, graphite-filled rings, solid for observed, dotted for inferred, dashed rings for external systems.
- **Do** set timestamps, IDs, metrics and commands in JetBrains Mono with tabular numbers.
- **Do** separate sections and rows with 1px hairlines and tonal steps rather than shadows.
- **Do** respect reduced motion: the live map stops advancing.

### Don't:
- **Don't** use gradients, glows, neon halos or coloured shadows (The Matte Rule).
- **Don't** use Interchange Blue as a fill, background or heading colour; it marks interaction only.
- **Don't** introduce a second display typeface or set headlines in mono.
- **Don't** put uppercase tracked labels or eyebrows above headings; the uppercase label cut is for table heads, form labels and badges only.
- **Don't** draw the product differently on the landing page than in the workspace; the demo reuses the workspace's map colours and card anatomy.
- **Don't** fabricate proof: the demo is labelled synthetic, and no customer logos, metrics or testimonials appear.
