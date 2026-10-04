# Layered, zoomable timeline — design

Status: proposed (2026-09-29). Replaces the bottom timeline bar of the workspace.

## Goal

A video-editor style timeline: a time ruler, stacked tracks (layers) and a playhead. It zooms
from the whole loaded range down to single milliseconds, so slow replay (0.001×) can be followed
span by span. It only shows data we already have; nothing is estimated.

## Tracks (top to bottom)

| Track | Source | Coarse zoom | Fine zoom |
|---|---|---|---|
| Actions | `GET …/actions` (plans with `started_at`) | bar from `started_at` to `finished_at` (or now if running), colour by state | same, with title |
| Events | engine events | count per bucket (bars) | individual ticks. Rare types (not `request.*`) are always drawn as labelled markers |
| Errors | engine spans with `is_error`, logs with severity ≥ 17 | count per bucket (red bars) | ticks |
| Requests | engine server/root spans (kind 2, 5, or spans without a parent) by start time | count per bucket (bars) | ticks |
| Latency p95 | durations of the same spans, by end time | p95 per bucket (area line). Empty buckets leave gaps, never 0 | same |
| One track per service | engine spans of that service | span count per bucket (heat bars) | each span is a clip `[start, end]`, red on error. Nested spans stack in sub-rows |

"Fine zoom" means a bucket holds at most 1 item on average, or the service track has at most 400
spans in view. Items beyond a cap are counted and shown as "+N not drawn".

## Interaction

- The **view window** `[viewFrom, viewTo]` sits inside the loaded range `[rangeFrom, rangeTo]`.
- **Zoom:** Ctrl+wheel or plain wheel over the tracks zooms around the pointer. The `+`, `−` and
  "Fit" buttons zoom around the playhead. The narrowest window is 1 ms, the widest is the loaded
  range.
- **Pan:** drag on the tracks, or Shift+wheel. The overview strip under the tracks shows the whole
  loaded range and the view window as a draggable frame.
- **Seek:** a click on the ruler or tracks moves the playhead (same as today's slider). The
  "Selected time" range input is kept as an accessible control.
- **Follow:** in live and replay, the view follows the playhead when it leaves the window, and
  "Fit" returns to the whole range.
- **Ruler:** tick spacing is chosen from 1 ms, 2, 5, 10, … up to 1 h. Labels read `HH:MM:SS`,
  `SS.mmm` or `.mmm µs`, depending on zoom.
- Transport controls, speed, mode and Mark A / Compare stay in the header row. Everything the
  e2e specs use keeps its accessible name.

## Architecture

- New pure function in `karbird-engine-core`:
  `timeline_tracks(from_ns, to_ns, buckets, clip_cap) -> TimelineTracks`.
  - It works on spans, logs and events already in memory, and honours the service filters.
  - Per-bucket arrays: `requests`, `errors`, `events`, `latency_p95_ms` (NaN = no data), and per
    service `counts`.
  - Clips for services when they are under the cap: service index, start and end relative to
    `from` in ms (f32), depth and flags.
  - Rare event markers: time and type.
  - Nothing is cached across calls. Cost is O(items in window); spans come from a start-sorted
    index built lazily and dropped on change, the same way as the hop index.
- WASM: `timelineTracks(fromNs, toNs, buckets, clipCap)` returns typed arrays and a small JSON
  header. Worker request/response use the existing RPC; the typed arrays are transferred.
- As built (2026-09-29): the engine scans all loaded spans on each call (no start-sorted index),
  `latency_p95_ms` uses `null` instead of NaN, clip times are f64 ms, and the WASM call returns
  serde JSON rather than typed arrays. Measured cost was not a problem at demo volume; revisit if
  the 5M-span budget is approached. The UI lives in `components/workspace/timeline/`
  (`TimelineTracksView.tsx`, `draw.ts`, `scale.ts`) inside the existing `TimelineBar`.
- UI: `TimelinePanel` (header and controls), `TimelineCanvas` (a 2D canvas draws the ruler,
  tracks and playhead; DOM holds the labels and the accessible controls) and a pure
  `timeline-scale.ts` (ticks, zoom and pan math, with vitest).
  - Track data is refetched when the view window, the range or the data generation changes,
    throttled to 10 per second. Playhead movement alone does not refetch.
- Actions track: react-query of the existing actions list, filtered to the view window.

## Honesty and limits

- Tracks cover only the loaded range. When the load was truncated, the panel says so.
- p95 is computed per bucket from the recorded spans only, and the tooltip shows the sample count.
- Requests counts server/root spans, not client spans, so a call is not counted twice.

## Testing

- Rust unit tests: bucket counts, p95 against a hand-computed set, fine-zoom clips with
  exact times, the cap, and filter handling.
- Vitest: tick selection across zoom levels, zoom around a point that stays fixed, pan clamping.
- e2e: the existing timeline specs keep passing. New checks: the timeline region shows the track
  labels, and zooming in changes the ruler to millisecond labels.
- Then `just check`, `just test`, `just wasm`, `just codegen` and `just test-e2e`.
