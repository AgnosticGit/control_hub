# Service metric cards on System Map

## Goal

Add an optional **Service metrics** map layer. When enabled, service cards show useful live or
selected-time resource and traffic signals in a compact expanded design based on the supplied
dark segmented-bar reference. Turning the layer off preserves the current compact card appearance.

## Existing data and boundaries

- The Karbird Collector already exports host CPU, memory, disk and network metrics, and per-container
  CPU, memory and network counters. The infrastructure API already maps these onto host/container
  records and associates containers with service IDs when the telemetry identifies them.
- Service request counts and calls between services come from spans and the map engine's node/edge
  frame data, not from the Collector. The Collector remains responsible for host/container metrics.
- The map currently draws 172×56 cards in PixiJS. Infrastructure frames and replica placements use
  the same card dimensions. Layer preferences are persisted per project/environment.

## User experience

- Add **Service metrics** to the existing Map layers menu. Its state is saved with other map layers.
- With the layer enabled, service cards expand to fit: service name/status; CPU and RAM segmented
  bars; container network receive/transmit rates; incoming/outgoing request rates. Keep the current
  service identity, status, selection, replica and external styling.
- Bars follow the supplied reference: dark unfilled segments, filled segments progressing from
  green through yellow to red as the measured share approaches its limit. Initial thresholds are
  green below 70%, yellow from 70% to below 90%, and red at 90% or above. Show the exact value and
  denominator in the card or its accessible description/tooltip.
- Hide unavailable metrics or show an em dash; never represent missing data as zero. Zoom continues
  to scale the card and labels together, and selection/fade behavior remains consistent.
- For a service with replicas, show aggregate CPU/RAM/network and request rates on its logical
  service card. The inspector retains per-container/per-machine breakdown. Aggregate ratios are
  calculated from summed usage divided by summed known capacity, not by averaging replica ratios.
  If a replica lacks the needed measurement, indicate partial coverage rather than presenting the
  aggregate as complete.

## Metric semantics and limits

- CPU: display container CPU usage as a share of its machine's logical CPU capacity using the
  existing normalized CPU values. Label that denominator explicitly; do not call it a container
  quota percentage unless a reliable container CPU quota is collected.
- RAM: use container memory usage divided by its reported memory limit when available; otherwise
  use machine memory capacity and label the denominator. Missing limits must not imply a full
  container allocation.
- Network: show receive/transmit bytes per second. Do not apply saturation colors until a reliable
  network bandwidth limit is available; the current Collector data does not establish one.
- Requests: derive incoming/outgoing calls per minute from the existing service-to-service edge
  traffic for the displayed time window. These values describe observed instrumented calls, not
  every packet or uninstrumented request.
- Disk currently exists at machine level. Do not attribute host disk usage to an individual service
  or container; per-container disk is out of scope until a trustworthy metric is collected.

## Data flow and implementation boundaries

- Reuse the existing infrastructure snapshot transport and existing frame buffers; do not add a new
  polling loop or duplicate telemetry request. Load the infrastructure snapshot when either the
  Infrastructure or Service metrics layer needs it, but draw machine frames only when Infrastructure
  is enabled.
- Pass the snapshot and layer visibility into the map renderer. Use the snapshot's container/service
  links for resource values and the current topology/frame edge data for request flows.
- Resize card drawing, hit testing, edge routing bounds, replica cards and machine-frame margins
  consistently. Reserve enough layout clearance for expanded cards so toggling the layer cannot
  create card/line overlaps. Keep the compact visual state when the layer is off.
- Do not change OTLP contracts or add a new backend endpoint. If inspection during implementation
  finds a missing metric or an inaccurate denominator in the existing Collector/API path, stop and
  revise this design before changing that contract.

## Verification

- Unit tests cover metric aggregation across replicas, partial coverage, missing/zero capacities,
  percentage thresholds, byte-rate formatting, and inbound/outbound edge aggregation.
- Renderer tests verify compact/expanded geometry, replica-card consistency, and that hit areas and
  routed edges use the expanded bounds.
- Playwright verifies the layer toggle persists after reload, expanded service cards display values,
  and disabling the layer restores compact cards.
- Run `just check`, `just test`, and UI integration/e2e checks with `just dev` and `just demo` running.

## Open implementation check

The current Collector/API model exposes memory limits but not an explicit CPU quota. The first
version therefore labels CPU as a share of host logical CPU capacity. If container-quota percentage
is required, that is a separate Collector/data-contract addition and must be designed and tested
before claiming quota-based saturation.
