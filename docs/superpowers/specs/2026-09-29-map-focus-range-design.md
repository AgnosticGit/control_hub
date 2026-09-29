# Map routing, service focus and replay range — design

Date: 2026-09-29. Status: approved by the user in chat ("Делай").

## Goals

1. Map lines do not pass through cards and do not lie on top of each other; crossings are
   minimised (zero is not always possible for non-planar graphs).
2. Services can be switched off on the timeline; switched-off services are dimmed on the
   timeline and on the map (with their lines and request dots). "All on" / "All off".
   Switching everything off and then two services on shows exactly their interaction.
3. The timeline tooltip sits directly above the pointer, clamped to the viewport; below the
   pointer only when there is no room above.
4. PAUSED has a neutral colour, distinct from the amber "Degraded".
5. A replay range can be selected on the timeline; Play loops inside it until the range is
   cleared with ✕ (or Esc), which also resets the view to the whole range.

## Decisions (asked and answered)

- Switched off = **dimmed**, not hidden (both views).
- Range gesture: ruler drag = cursor; **drag across the rows = select range**; a click on the
  rows = place the cursor.
- Lines: **new layered layout + routing** (node order may change once when a node appears).

## Design

### Engine (`controlhub-engine-core`)

- `Filters.muted_services: Vec<String>` (node IDs, `serde(default)`).
- A node is dimmed when it is a muted service, or a resource/dependency whose every caller is
  dimmed. An edge is dimmed when either end is dimmed; its hops carry `HOP_DIMMED`.
  New flags: `FLAG_DIMMED = 32` (nodes and edges), `HOP_DIMMED = 16`.
- `timeline_tracks`: service rows stay; the Requests/Errors/Latency/Events aggregates count
  only services that are not muted.
- Layout (`layout.rs`) becomes a layered (Sugiyama-style) layout, recomputed whenever the node
  or edge set changes:
  - layers by longest path (cycle-safe, as before);
  - edges spanning more than one layer get dummy nodes in the intermediate layers;
  - order inside layers by barycenter sweeps (down and up), keeping the order with the fewest
    crossings; deterministic tie-breaks by ID;
  - y coordinates by iterative neighbour averaging under minimum separations (card–card,
    card–dummy, dummy–dummy), so reserved dummy slots are free corridors.
- `TopologyViewEdge.points: Vec<[f32; 2]>`: the dummy waypoints of the edge (empty for edges
  between adjacent layers). Regenerated TS/OpenAPI types (`just codegen`).

### Renderer (`apps/web/src/renderer`)

- `edge-route.ts` (pure, tested) builds each edge's polyline from source card → waypoints →
  target card:
  - ports: edges leaving (entering) the same card side are spread along the side, sorted by the
    other end's y;
  - through a dummy waypoint the line crosses the column band horizontally at the reserved y;
  - between two stations in adjacent columns the connector follows the chosen style (curved
    S-bezier, square with the vertical in the gap, metro 45°);
  - vertical runs of square/metro connectors in the same gap get separate lanes (interval
    colouring, 6 px apart).
  - Backward and same-layer edges fall back to `edgePath`.
- Dimmed nodes, edges and hops are drawn at about 25 % alpha.

### Web UI

- Workspace state `muted: string[]` → engine filters and timeline tracks.
- Timeline service rows get a switch; the services group header has "All on" / "All off";
  muted rows are drawn dimmed.
- Tooltip: `position: fixed`, pure placement function (above, centred, clamped, flip below).
- Mode colours: paused uses a neutral slate colour.
- Replay range: `loop: {from, to} | null` in the engine hook. Replay wraps from `to` to `from`.
  Play outside the range starts at `from`. The range is drawn as a band with draggable edges;
  a header chip shows it with ✕; Esc clears it; clearing resets the view to the whole range.

## Tests

- Engine: dimming rules; timeline aggregates without muted services; layout on the demo
  topology has zero crossings, no segment through a card, deterministic output; long edges get
  waypoints.
- vitest: routing (ports, lanes, no segment inside a card), tooltip placement, loop wrap.
- e2e: service switches and All on/off, range selection and ✕, paused badge colour.
