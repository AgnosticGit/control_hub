// PixiJS (WebGL) renderer for the service map. It only draws: positions,
// statuses, flags and request hops come precomputed from the engine frame
// buffers. Pan and zoom are local; the resulting world-space viewport is
// reported back so the engine can cull and choose a level of detail.

// Replaces Pixi's `new Function` code generation so the page runs without CSP 'unsafe-eval'.
import "pixi.js/unsafe-eval";
import { Application, Container, Graphics, Text, TextStyle } from "pixi.js";
import {
  EDGE_STRIDE,
  FLAG_DIMMED,
  FLAG_EXTERNAL,
  FLAG_HIGHLIGHT,
  FLAG_INFERRED,
  FLAG_PROBLEM,
  FLAG_RESOURCE,
  HOP_DIMMED,
  HOP_ERROR,
  HOP_HIGHLIGHT,
  HOP_LEAF,
  HOP_STRIDE,
  NODE_STRIDE,
  type EntityKindDto,
  type Frame,
  type InfraView,
  type TopologyView,
} from "@karbird/web-engine";
import { FRAME_HEADER, NOISY_LINK, chipAt, frameAt, frameLook, linkArc, linkLabelAt } from "./infra-layer";
import { DEFAULT_EDGE_STYLE, build, dashes, edgePath, endDirection, pointAt, type EdgePath, type EdgeStyle } from "./edge-path";
import { edgeAt, midpoint } from "./edge-hit";
import { routeEdges, type RouteEdge, type RouteNode } from "./edge-route";
import { dashedRect, nodeSubLabel } from "./external";
import { ACCENT, CARD_BG, EDGE_ERROR, EDGE_INFERRED, EDGE_OK, EDGE_WARN, EXTERNAL_COLOR, STATUS_COLORS } from "./map-colors";
import { cullRect } from "./cull";
import { fitCamera } from "./fit";
import { gridLines } from "./grid";
import { animTimeMs, hopPhase, type AnimClock } from "./hops";
import { serviceColors } from "./service-colors";
import { frameFocused, nodeFocused, SELECTION_DIM_ALPHA, selectionAlpha, type MapSelection } from "./selection-style";

/** Pan (screen px) and zoom of the map. */
export interface MapCamera {
  x: number;
  y: number;
  scale: number;
}

export interface MapCallbacks {
  onViewport(x: number, y: number, width: number, height: number, scale: number): void;
  onPick(worldX: number, worldY: number): void;
  /** A click on a machine frame (infrastructure layer) outside any service card. */
  onPickHost?(hostId: string): void;
  /** A click on a service instance's replica card inside a machine frame. */
  onPickInstance?(instanceId: string, hostId: string, serviceId: string): void;
  /** A click on a line between two cards (engine edge index). */
  onPickEdge?(index: number): void;
}

/** How close (screen px) a click must land to a line to select it. */
const EDGE_HIT_PX = 7;
const EDGE_SELECTED = 0x22d3ee;

const MIN_SCALE = 0.15;
const MAX_SCALE = 4;
/** Node card size in world units (layout spacing is 300 × 140). */
const CARD_W = 172;
const CARD_H = 56;
const CARD_BOX = { hw: CARD_W / 2 + 4, hh: CARD_H / 2 + 4 };
// Ports use the whole card side but its rounded corners, so many edges stay apart.
const ROUTE_BOX = { ...CARD_BOX, portSpan: CARD_H - 12 };
const LINE_STYLE = { join: "round", cap: "round" } as const;
/** Opacity of switched-off services and everything attached to them. */
const DIM_ALPHA = 0.22;
const GRID_COLOR = 0x94a3b8;
const GRID_MINOR_ALPHA = 0.04;
const GRID_MAJOR_ALPHA = 0.08;

interface EdgeGeom {
  path: EdgePath;
  color: number;
}

/** A routed edge path and the end positions it was routed for. */
interface Route {
  path: EdgePath;
  sx: number;
  sy: number;
  tx: number;
  ty: number;
}

interface NodeText {
  name: Text;
  sub: Text;
  subValue: string;
}

interface ReplicaText extends NodeText {
  count: Text;
}

export class PixiMap {
  private app: Application | null = null;
  private host: HTMLElement | null = null;
  private readonly world = new Container();
  private readonly grid = new Graphics();
  private gridKey = "";
  private readonly edges = new Graphics();
  private readonly edgesDim = new Graphics({ alpha: DIM_ALPHA });
  private readonly hops = new Graphics();
  private readonly nodes = new Graphics();
  private readonly nodesDim = new Graphics({ alpha: DIM_ALPHA });
  private readonly replicaCards = new Graphics();
  private readonly replicaLabels = new Container();
  private readonly replicaTextPool = new Map<string, ReplicaText>();
  private readonly labels = new Container();
  private readonly textPool = new Map<number, NodeText>();
  /** Infrastructure layer: machine frames below the cards, links and badges above them. */
  private readonly infraBack = new Graphics();
  private readonly infraFront = new Graphics();
  private readonly infraLabels = new Container();
  private infraView: InfraView | null = null;
  private selectedHost: string | null = null;
  private readonly frameStyle = new TextStyle({
    fill: 0xc4b5fd,
    fontSize: 13,
    fontWeight: "600",
    fontFamily: "'JetBrains Mono', ui-monospace, monospace",
  });
  private readonly frameHotStyle = new TextStyle({
    fill: 0xfca5a5,
    fontSize: 13,
    fontWeight: "600",
    fontFamily: "'JetBrains Mono', ui-monospace, monospace",
  });
  private readonly chipStyle = new TextStyle({ fill: 0xe6ecf5, fontSize: 11, fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif" });
  private readonly linkStyle = new TextStyle({ fill: 0xfbbf24, fontSize: 12, fontWeight: "600", fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif" });
  private readonly nameStyle = new TextStyle({
    fill: 0xe6ecf5,
    fontSize: 13,
    fontWeight: "600",
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif",
  });
  private readonly subStyle = new TextStyle({
    fill: 0x7d8aa5,
    fontSize: 11,
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif",
  });
  private frame: Frame | null = null;
  private frameTimeUs = 0;
  private frameArrivedAt = 0;
  private clock: AnimClock = { kind: "frame", rate: 0 };
  private edgeGeom = new Map<number, EdgeGeom>();
  private edgeStyle: EdgeStyle = DEFAULT_EDGE_STYLE;
  private routeNodes: RouteNode[] = [];
  private routeEdges: RouteEdge[] = [];
  private readonly hostByNode = new Map<number, string>();
  private routes = new Map<number, Route>();
  private names: string[] = [];
  private kinds: EntityKindDto[] = [];
  private colors = new Map<number, number>();
  private selectedIndex: number | null = null;
  private selectedEdge: number | null = null;
  private selectedEdgeNodes: readonly [number, number] | null = null;
  private edgeAnchors = "";
  private fitted = false;
  /** The camera is where `fit` put it (no pan or zoom since). */
  private cameraIsFit = false;
  /** Screen pixels at the top covered by an overlay; `fit` keeps the map below them. */
  private topInset = 0;
  private reducedMotion = false;
  private destroyed = false;
  private cleanup: Array<() => void> = [];

  constructor(private readonly callbacks: MapCallbacks) {}

  /** Throws when WebGL is unavailable; the caller then shows the table view. */
  async mount(host: HTMLElement) {
    const app = new Application();
    await app.init({
      resizeTo: host,
      antialias: true,
      backgroundAlpha: 0,
      preference: "webgl",
      autoDensity: true,
      resolution: Math.min(window.devicePixelRatio || 1, 2),
    });
    if (this.destroyed) {
      app.destroy(true);
      return;
    }
    this.app = app;
    this.host = host;
    app.canvas.setAttribute("aria-hidden", "true");
    host.appendChild(app.canvas);
    // Hops above cards so a request waiting at its callee stays visible on the card border.
    this.world.addChild(
      this.infraBack,
      this.edgesDim,
      this.nodesDim,
      this.edges,
      this.nodes,
      this.replicaCards,
      this.infraFront,
      this.hops,
      this.labels,
      this.replicaLabels,
      this.infraLabels,
    );
    this.drawInfra();
    app.stage.addChild(this.grid, this.world);
    app.ticker.add(() => this.drawGrid());
    this.reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    // With reduced motion hops are drawn once per frame (no interpolation between frames).
    if (!this.reducedMotion) app.ticker.add(() => this.drawHops());
    this.bindInput(app.canvas);
    const ro = new ResizeObserver(() => this.reportViewport());
    ro.observe(host);
    this.cleanup.push(() => ro.disconnect());
  }

  setTopology(topology: TopologyView | null) {
    const nodes = topology?.nodes ?? [];
    const names: string[] = [];
    const kinds: EntityKindDto[] = [];
    for (const n of nodes) {
      names[n.index] = n.name;
      kinds[n.index] = n.kind;
    }
    this.kinds = kinds;
    this.colors = serviceColors(nodes);
    this.routeNodes = nodes.map((n) => ({ index: n.index, x: n.x, y: n.y }));
    this.routeEdges = (topology?.edges ?? []).map((e) => ({ index: e.index, source: e.source, target: e.target, points: e.points }));
    const selectedRoute = this.routeEdges.find((edge) => edge.index === this.selectedEdge);
    this.selectedEdgeNodes = selectedRoute ? [selectedRoute.source, selectedRoute.target] : null;
    this.reroute();
    const infra = topology?.infra ?? null;
    // Switching the layer moves every card: fit again on the next frame.
    const refit = (infra === null) !== (this.infraView === null);
    this.infraView = infra;
    this.hostByNode.clear();
    for (const node of this.routeNodes) {
      const frame = infra?.frames.find((candidate) => node.x >= candidate.x && node.x <= candidate.x + candidate.width && node.y >= candidate.y && node.y <= candidate.y + candidate.height);
      if (frame) this.hostByNode.set(node.index, frame.host_id);
    }
    this.drawInfra();
    if (refit) {
      if (this.frame) this.fit();
      else this.fitted = false;
    }
    if (names.length !== this.names.length || names.some((n, i) => n !== this.names[i])) {
      this.names = names;
      for (const t of this.textPool.values()) {
        t.name.destroy();
        t.sub.destroy();
      }
      this.textPool.clear();
      this.labels.removeChildren();
    }
    if (this.frame) {
      this.draw(this.frame);
      if (this.reducedMotion) this.drawHops();
    }
  }

  setSelected(index: number | null) {
    if (index === this.selectedIndex) return;
    this.selectedIndex = index;
    this.syncSelectionFade();
    this.drawInfra();
    if (this.frame) this.draw(this.frame);
  }

  /** Highlights the line open in the inspector (engine edge index). */
  setSelectedEdge(index: number | null) {
    if (index === this.selectedEdge) return;
    this.selectedEdge = index;
    const edge = index === null ? undefined : this.routeEdges.find((candidate) => candidate.index === index);
    this.selectedEdgeNodes = edge ? [edge.source, edge.target] : null;
    this.syncSelectionFade();
    this.host?.setAttribute("data-selected-edge", index === null ? "" : String(index));
    this.drawInfra();
    if (this.frame) this.draw(this.frame);
  }

  setEdgeStyle(style: EdgeStyle) {
    if (style === this.edgeStyle) return;
    this.edgeStyle = style;
    this.reroute();
    if (this.frame) {
      this.draw(this.frame);
      this.drawHops();
    }
  }

  /** How hop timestamps map onto wall time (live delay, replay rate or frozen). */
  setClock(clock: AnimClock) {
    this.clock = clock;
    this.frameArrivedAt = performance.now();
  }

  render(frame: Frame) {
    this.frame = frame;
    this.frameArrivedAt = performance.now();
    const t = frame.summary.time_ns;
    this.frameTimeUs = t ? Number(BigInt(t) / 1000n) : 0;
    if (!this.app) return;
    if (!this.fitted && frame.nodeIndices.length > 0) {
      this.fitted = true;
      this.fit();
    }
    this.draw(frame);
    if (this.reducedMotion) this.drawHops();
    // Which cards are drawn, for tests and debugging (node indices).
    this.host?.setAttribute("data-drawn-nodes", Array.from(frame.nodeIndices).join(","));
  }

  fit() {
    const app = this.app;
    const f = this.frame;
    if (!app || !f || f.nodeIndices.length === 0) return;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (let i = 0; i < f.nodeIndices.length; i++) {
      const x = f.nodeData[i * NODE_STRIDE] ?? 0;
      const y = f.nodeData[i * NODE_STRIDE + 1] ?? 0;
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x);
      maxY = Math.max(maxY, y);
    }
    for (const e of this.routeEdges) {
      for (const [, y] of e.points) {
        minY = Math.min(minY, y);
        maxY = Math.max(maxY, y);
      }
    }
    for (const fr of this.infraView?.frames ?? []) {
      minX = Math.min(minX, fr.x);
      minY = Math.min(minY, fr.y);
      maxX = Math.max(maxX, fr.x + fr.width);
      maxY = Math.max(maxY, fr.y + fr.height);
    }
    const cam = fitCamera(
      { minX, minY, maxX, maxY },
      {
        width: app.screen.width,
        height: app.screen.height,
        padX: CARD_W,
        padY: CARD_H * 2,
        topInset: this.topInset,
        insetGap: CARD_H,
        minScale: MIN_SCALE,
        maxScale: 1.3,
      },
    );
    this.world.scale.set(cam.scale);
    this.world.position.set(cam.x, cam.y);
    this.reportViewport();
    this.cameraIsFit = true;
    // Screen y of the top card edge after a fit, for tests: it must stay below any overlay.
    this.host?.setAttribute("data-fit-top", String(Math.round(cam.y + (minY - CARD_H / 2) * cam.scale)));
  }

  /** Highlights the frame of the machine open in the inspector. */
  setSelectedHost(hostId: string | null) {
    if (hostId === this.selectedHost) return;
    this.selectedHost = hostId;
    this.syncSelectionFade();
    this.drawInfra();
  }

  /** Keeps `fit` clear of an overlay this many screen pixels tall; refits a fitted camera. */
  setTopInset(px: number) {
    const next = Math.max(0, Math.round(px));
    if (next === this.topInset) return;
    this.topInset = next;
    if (this.cameraIsFit) this.fit();
  }

  zoomBy(factor: number, sx?: number, sy?: number) {
    const app = this.app;
    if (!app) return;
    const cx = sx ?? app.screen.width / 2;
    const cy = sy ?? app.screen.height / 2;
    const old = this.world.scale.x;
    const next = Math.max(MIN_SCALE, Math.min(MAX_SCALE, old * factor));
    const wx = (cx - this.world.x) / old;
    const wy = (cy - this.world.y) / old;
    this.world.scale.set(next);
    this.world.position.set(cx - wx * next, cy - wy * next);
    this.reportViewport();
  }

  getCamera(): MapCamera {
    return { x: this.world.x, y: this.world.y, scale: this.world.scale.x };
  }

  /** Restores a saved pan and zoom; the map then no longer fits itself to the first frame. */
  setCamera(camera: MapCamera) {
    this.fitted = true;
    this.world.scale.set(Math.max(MIN_SCALE, Math.min(MAX_SCALE, camera.scale)));
    this.world.position.set(camera.x, camera.y);
    this.reportViewport();
  }

  panBy(dx: number, dy: number) {
    this.world.position.set(this.world.x + dx, this.world.y + dy);
    this.reportViewport();
  }

  destroy() {
    this.destroyed = true;
    for (const c of this.cleanup) c();
    this.cleanup = [];
    this.app?.destroy(true, { children: true });
    this.app = null;
  }

  /** Routes are recomputed only when the topology or the line style changes, not per frame. */
  private reroute() {
    const paths = routeEdges(this.edgeStyle, this.routeNodes, this.routeEdges, ROUTE_BOX);
    const pos = new Map<number, RouteNode>();
    for (const n of this.routeNodes) pos.set(n.index, n);
    const routes = new Map<number, Route>();
    for (const e of this.routeEdges) {
      const path = paths.get(e.index);
      const s = pos.get(e.source);
      const t = pos.get(e.target);
      if (path && s && t) routes.set(e.index, { path, sx: s.x, sy: s.y, tx: t.x, ty: t.y });
    }
    this.routes = routes;
  }

  private reportViewport() {
    this.cameraIsFit = false;
    const app = this.app;
    if (!app) return;
    const s = this.world.scale.x;
    const r = cullRect({ x: -this.world.x / s, y: -this.world.y / s, width: app.screen.width / s, height: app.screen.height / s }, CARD_W / 2 + 8);
    this.callbacks.onViewport(r.x, r.y, r.width, r.height, s);
    this.publishEdgeAnchors();
    this.publishReplicaAnchors();
  }

  /** Positions of replica cards in canvas pixels (`service@host:x,y`) for UI tests/debugging. */
  private publishReplicaAnchors() {
    const s = this.world.scale.x;
    const anchors = (this.infraView?.chips ?? []).map((c) =>
      `${encodeURIComponent(c.service_id)}@${encodeURIComponent(c.host_id)}:${Math.round(c.x * s + this.world.x)},${Math.round(c.y * s + this.world.y)}`,
    ).join(";");
    this.host?.setAttribute("data-replica-anchors", anchors);
    const frames = (this.infraView?.frames ?? []).map((frame) =>
      `${encodeURIComponent(frame.host_id)}:${Math.round((frame.x + frame.width / 2) * s + this.world.x)},${Math.round((frame.y + FRAME_HEADER / 2) * s + this.world.y)}`,
    ).join(";");
    this.host?.setAttribute("data-frame-anchors", frames);
  }

  /**
   * Midpoints of the drawn lines in canvas pixels (`index:x,y;…`), for tests and debugging:
   * the canvas itself is opaque to the accessibility tree.
   */
  private publishEdgeAnchors() {
    const s = this.world.scale.x;
    const parts: string[] = [];
    for (const [index, g] of this.edgeGeom) {
      const m = midpoint(g.path.pts);
      if (m) parts.push(`${index}:${Math.round(m[0] * s + this.world.x)},${Math.round(m[1] * s + this.world.y)}`);
    }
    const next = parts.join(";");
    if (next === this.edgeAnchors) return;
    this.edgeAnchors = next;
    this.host?.setAttribute("data-edge-anchors", next);
  }

  /** Clicks anywhere on a card select it; the engine picks by the node centre. */
  private pickAt(wx: number, wy: number) {
    const f = this.frame;
    if (f) {
      for (let i = 0; i < f.nodeIndices.length; i++) {
        const x = f.nodeData[i * NODE_STRIDE] ?? 0;
        const y = f.nodeData[i * NODE_STRIDE + 1] ?? 0;
        if (Math.abs(wx - x) <= CARD_W / 2 && Math.abs(wy - y) <= CARD_H / 2) {
          this.callbacks.onPick(x, y);
          return;
        }
      }
    }
    const chip = chipAt(this.infraView, wx, wy);
    if (chip) {
      if (this.callbacks.onPickInstance) this.callbacks.onPickInstance(chip.instance_id, chip.host_id, chip.service_id);
      else this.callbacks.onPick(chip.x, chip.y);
      return;
    }
    if (this.callbacks.onPickEdge) {
      const paths = Array.from(this.edgeGeom, ([i, g]) => [i, g.path] as const);
      const edge = edgeAt(paths, wx, wy, EDGE_HIT_PX / this.world.scale.x);
      if (edge !== null) {
        this.callbacks.onPickEdge(edge);
        return;
      }
    }
    const machine = frameAt(this.infraView, wx, wy);
    if (machine && this.callbacks.onPickHost) {
      this.callbacks.onPickHost(machine.host_id);
      return;
    }
    this.callbacks.onPick(wx, wy);
  }

  /** Static per topology: redrawn only when the layout or the snapshot changes. */
  private drawInfra() {
    const back = this.infraBack;
    const front = this.infraFront;
    back.clear();
    front.clear();
    for (const t of this.infraLabels.removeChildren()) t.destroy();
    const view = this.infraView;
    this.host?.setAttribute("data-infra-frames", view ? view.frames.map((f) => f.host_id).join(",") : "");
    const selected = view?.frames.some((f) => f.host_id === this.selectedHost) ? this.selectedHost : null;
    this.host?.setAttribute("data-infra-selected", selected ?? "");
    const selection = this.selection();
    this.host?.setAttribute("data-selection-dimmed", String(selectionAlpha(selection) < 1));
    const selectedHosts = new Set<string>();
    if (selection.node !== null) {
      const primaryHost = this.hostForNode(selection.node);
      if (primaryHost) selectedHosts.add(primaryHost);
      for (const c of view?.chips ?? []) if (c.node === selection.node) selectedHosts.add(c.host_id);
    }
    this.host?.setAttribute("data-selected-replicas", Array.from(selectedHosts).sort().join(","));
    if (!view) return;
    for (const f of view.frames) {
      const look = frameLook(f, f.host_id === selected);
      const fade = frameFocused(f.host_id, selection) ? 1 : SELECTION_DIM_ALPHA;
      const color = look.color;
      if (look.halo > 0) {
        const h = look.halo;
        back.roundRect(f.x - h, f.y - h, f.width + 2 * h, f.height + 2 * h, 18 + h).stroke({ color, width: h, alpha: 0.18 });
      }
      back
        .roundRect(f.x, f.y, f.width, f.height, 18)
        .fill({ color, alpha: look.fillAlpha * fade })
        .stroke({ color, width: look.strokeWidth, alpha: look.strokeAlpha * fade });
      back.roundRect(f.x, f.y, f.width, FRAME_HEADER - 8, 18).fill({ color, alpha: look.headerAlpha * fade });
      const label = new Text({ text: f.label, style: f.hot ? this.frameHotStyle : this.frameStyle });
      label.position.set(f.x + 16, f.y + 10);
      label.alpha = fade;
      this.infraLabels.addChild(label);
    }
    const pos = new Map<number, RouteNode>();
    for (const n of this.routeNodes) pos.set(n.index, n);
    for (const b of view.badges) {
      const n = pos.get(b.node);
      if (!n) continue;
      const secondaryCount = view.chips.filter((c) => c.node === b.node).reduce((total, c) => total + c.instance_ids.length, 0);
      const localCount = b.count - secondaryCount;
      if (localCount <= 1) continue;
      const t = new Text({ text: `×${localCount}`, style: this.chipStyle });
      const w = t.width + 14;
      const x = n.x + CARD_W / 2 - w + 6;
      const y = n.y - CARD_H / 2 - 11;
      const fade = nodeFocused(b.node, this.hostForNode(b.node), selection) ? 1 : SELECTION_DIM_ALPHA;
      front.roundRect(x, y, w, 20, 10).fill({ color: 0x4c1d95, alpha: 0.95 * fade }).stroke({ color: 0xa78bfa, width: 1, alpha: fade });
      t.alpha = fade;
      t.position.set(x + 7, y + 3);
      this.infraLabels.addChild(t);
    }
    this.publishReplicaAnchors();
    for (const l of view.links) {
      for (const dash of dashes(build(linkArc(l.from, l.to)), 8, 6)) line(front, dash);
      front.stroke({ color: NOISY_LINK, width: 3, alpha: 0.95 * selectionAlpha(selection), ...LINE_STYLE });
      front.circle(l.to[0], l.to[1], 6).stroke({ color: NOISY_LINK, width: 2 });
      const [mx, my] = linkLabelAt(l.from, l.to);
      const t = new Text({ text: l.label, style: this.linkStyle });
      const fade = selectionAlpha(selection);
      front.roundRect(mx - t.width / 2 - 6, my - 10, t.width + 12, 20, 6).fill({ color: CARD_BG, alpha: 0.95 * fade }).stroke({ color: NOISY_LINK, width: 1, alpha: fade });
      t.position.set(mx - t.width / 2, my - 8);
      t.alpha = fade;
      this.infraLabels.addChild(t);
    }
  }

  private bindInput(canvas: HTMLCanvasElement) {
    let dragging = false;
    let moved = 0;
    let lastX = 0;
    let lastY = 0;
    const down = (e: PointerEvent) => {
      dragging = true;
      moved = 0;
      lastX = e.clientX;
      lastY = e.clientY;
      canvas.setPointerCapture(e.pointerId);
    };
    const move = (e: PointerEvent) => {
      if (!dragging) return;
      const dx = e.clientX - lastX;
      const dy = e.clientY - lastY;
      moved += Math.abs(dx) + Math.abs(dy);
      lastX = e.clientX;
      lastY = e.clientY;
      this.world.position.set(this.world.x + dx, this.world.y + dy);
      this.reportViewport();
    };
    const up = (e: PointerEvent) => {
      if (!dragging) return;
      dragging = false;
      canvas.releasePointerCapture(e.pointerId);
      if (moved < 4) {
        const rect = canvas.getBoundingClientRect();
        const s = this.world.scale.x;
        this.pickAt((e.clientX - rect.left - this.world.x) / s, (e.clientY - rect.top - this.world.y) / s);
      } else {
        this.reportViewport();
      }
    };
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = canvas.getBoundingClientRect();
      this.zoomBy(Math.exp(-e.deltaY * 0.0015), e.clientX - rect.left, e.clientY - rect.top);
    };
    canvas.addEventListener("pointerdown", down);
    canvas.addEventListener("pointermove", move);
    canvas.addEventListener("pointerup", up);
    canvas.addEventListener("wheel", wheel, { passive: false });
    this.cleanup.push(() => {
      canvas.removeEventListener("pointerdown", down);
      canvas.removeEventListener("pointermove", move);
      canvas.removeEventListener("pointerup", up);
      canvas.removeEventListener("wheel", wheel);
    });
  }

  /** Screen-space grid under the map; redrawn only when pan, zoom or the canvas size change. */
  private drawGrid() {
    const app = this.app;
    if (!app) return;
    const { width, height } = app.screen;
    const s = this.world.scale.x;
    const key = `${this.world.x}|${this.world.y}|${s}|${width}|${height}`;
    if (key === this.gridKey) return;
    this.gridKey = key;
    const g = this.grid;
    g.clear();
    const xs = gridLines(this.world.x, s, width);
    const ys = gridLines(this.world.y, s, height);
    for (const major of [false, true]) {
      for (const l of xs) if (l.major === major) g.moveTo(Math.round(l.pos) + 0.5, 0).lineTo(Math.round(l.pos) + 0.5, height);
      for (const l of ys) if (l.major === major) g.moveTo(0, Math.round(l.pos) + 0.5).lineTo(width, Math.round(l.pos) + 0.5);
      g.stroke({ color: GRID_COLOR, width: 1, alpha: major ? GRID_MAJOR_ALPHA : GRID_MINOR_ALPHA });
    }
  }

  private draw(f: Frame) {
    this.drawEdges(f);
    this.drawNodes(f);
    this.drawReplicaCards(f);
  }

  /** Secondary placements reuse the same logical node styling at full service-card size. */
  private drawReplicaCards(f: Frame) {
    this.replicaCards.clear();
    const selection = this.selection();
    const seen = new Set<string>();
    const dataByNode = new Map<number, number>();
    for (let i = 0; i < f.nodeIndices.length; i++) dataByNode.set(f.nodeIndices[i] ?? 0, i * NODE_STRIDE);
    for (const c of this.infraView?.chips ?? []) {
      const offset = dataByNode.get(c.node);
      if (offset === undefined) continue;
      const data = f.nodeData;
      const status = data[offset + 3] ?? 0;
      const active = data[offset + 4] ?? 0;
      const requests = data[offset + 5] ?? 0;
      const flags = data[offset + 6] ?? 0;
      const statusColor = STATUS_COLORS[status] ?? STATUS_COLORS[0] ?? 0x64748b;
      const external = (flags & FLAG_EXTERNAL) !== 0;
      const color = status >= 2 ? statusColor : external ? EXTERNAL_COLOR : (this.colors.get(c.node) ?? statusColor);
      const focused = nodeFocused(c.node, c.host_id, selection);
      const fade = focused ? 1 : SELECTION_DIM_ALPHA;
      const key = `${c.service_id}@${c.host_id}`;
      seen.add(key);
      const left = c.x - CARD_W / 2;
      const top = c.y - CARD_H / 2;
      const g = this.replicaCards;
      if (status > 0) g.roundRect(left - 3, top - 3, CARD_W + 6, CARD_H + 6, 15).stroke({ color, width: 6, alpha: (status >= 2 ? 0.22 : 0.1) * fade });
      g.roundRect(left, top, CARD_W, CARD_H, 12).fill({ color: CARD_BG, alpha: 0.96 * fade }).stroke({ color, width: 1.5, alpha: 0.85 * fade });
      if ((flags & FLAG_HIGHLIGHT) !== 0) g.roundRect(left - 5, top - 5, CARD_W + 10, CARD_H + 10, 16).stroke({ color: ACCENT, width: 2 });
      if (c.node === this.selectedIndex) g.roundRect(left - 8, top - 8, CARD_W + 16, CARD_H + 16, 18).stroke({ color: 0xffffff, width: 2, alpha: 0.9 });
      g.roundRect(left + 12, c.y - 15, 30, 30, 9).fill({ color, alpha: 0.14 });
      drawKindIcon(g, this.kinds[c.node], (flags & FLAG_RESOURCE) !== 0, left + 27, c.y, color);
      g.circle(left + CARD_W - 12, top + 12, 3.5).fill({ color: statusColor, alpha: status > 0 ? 1 : 0.6 });
      let pooled = this.replicaTextPool.get(key);
      if (!pooled) {
        pooled = {
          name: new Text({ text: truncate(this.names[c.node] ?? c.label, 17), style: this.nameStyle }),
          sub: new Text({ text: "", style: this.subStyle }),
          subValue: "",
          count: new Text({ text: "", style: this.chipStyle }),
        };
        this.replicaTextPool.set(key, pooled);
        this.replicaLabels.addChild(pooled.name, pooled.sub, pooled.count);
      }
      const subValue = nodeSubLabel(external, active, requests);
      if (subValue !== pooled.subValue) {
        pooled.sub.text = subValue;
        pooled.subValue = subValue;
      }
      const nameValue = truncate(this.names[c.node] ?? c.label, 17);
      if (pooled.name.text !== nameValue) pooled.name.text = nameValue;
      pooled.name.position.set(left + 52, c.y - 17);
      pooled.sub.position.set(left + 52, c.y + 2);
      pooled.name.alpha = fade;
      pooled.sub.alpha = fade;
      if (c.instance_ids.length > 1) {
        pooled.count.text = `×${c.instance_ids.length}`;
        pooled.count.visible = true;
        const width = pooled.count.width + 12;
        const x = left + CARD_W - width - 6;
        const y = top + 5;
        g.roundRect(x, y, width, 18, 9).fill({ color: 0x4c1d95, alpha: 0.95 * fade }).stroke({ color: 0xa78bfa, width: 1, alpha: fade });
        pooled.count.position.set(x + 6, y + 2);
        pooled.count.alpha = fade;
      } else {
        pooled.count.visible = false;
      }
    }
    for (const [key, text] of this.replicaTextPool) {
      if (!seen.has(key)) {
        text.name.visible = false;
        text.sub.visible = false;
        text.count.visible = false;
      } else {
        text.name.visible = true;
        text.sub.visible = true;
      }
    }
  }

  private selection(): MapSelection {
    return { node: this.selectedIndex, edge: this.selectedEdge, edgeNodes: this.selectedEdgeNodes, host: this.selectedHost };
  }

  private syncSelectionFade() {
    this.hops.alpha = selectionAlpha(this.selection());
  }

  /** Logical service cards belong to exactly one machine frame in infrastructure mode. */
  private hostForNode(node: number): string | null {
    return this.hostByNode.get(node) ?? null;
  }

  private drawEdges(f: Frame) {
    this.edges.clear();
    this.edgesDim.clear();
    this.edgeGeom.clear();
    const selection = this.selection();
    for (let i = 0; i < f.edgeIndices.length; i++) {
      const o = i * EDGE_STRIDE;
      const sx = f.edgeData[o] ?? 0;
      const sy = f.edgeData[o + 1] ?? 0;
      const tx = f.edgeData[o + 2] ?? 0;
      const ty = f.edgeData[o + 3] ?? 0;
      const calls = f.edgeData[o + 5] ?? 0;
      const ratio = f.edgeData[o + 6] ?? 0;
      const flags = f.edgeData[o + 7] ?? 0;
      const inferred = (flags & FLAG_INFERRED) !== 0;
      const highlight = (flags & FLAG_HIGHLIGHT) !== 0;
      const color = highlight
        ? ACCENT
        : (flags & FLAG_PROBLEM) !== 0
          ? ratio >= 0.5
            ? EDGE_ERROR
            : EDGE_WARN
          : (flags & FLAG_EXTERNAL) !== 0
            ? EXTERNAL_COLOR
            : inferred
              ? EDGE_INFERRED
              : EDGE_OK;
      const index = f.edgeIndices[i] ?? 0;
      const fade = this.selectedEdge === index || selectionAlpha(selection) === 1 ? 1 : SELECTION_DIM_ALPHA;
      const route = this.routes.get(index);
      // A route is only valid for the positions it was computed for (topology may lag a frame).
      const fresh = route && near(route.sx, sx) && near(route.sy, sy) && near(route.tx, tx) && near(route.ty, ty);
      const path = fresh ? route.path : edgePath(this.edgeStyle, sx, sy, tx, ty, CARD_BOX);
      if (!path) continue;
      this.edgeGeom.set(index, { path, color });
      const e = (flags & FLAG_DIMMED) !== 0 ? this.edgesDim : this.edges;
      if (index === this.selectedEdge) line(this.edges, path.pts).stroke({ color: EDGE_SELECTED, width: 10, alpha: 0.35, ...LINE_STYLE });
      const width = highlight ? 2.5 : 1.25 + Math.min(1.5, Math.log10(1 + calls) * 0.6);
      if (inferred) {
        for (const dash of dashes(path, 6, 5)) line(e, dash);
        e.stroke({ color, width, alpha: 0.8 * fade, ...LINE_STYLE });
      } else {
        line(e, path.pts).stroke({ color, width: width + 6, alpha: (calls > 0 ? 0.1 : 0.05) * fade, ...LINE_STYLE });
        line(e, path.pts).stroke({ color, width, alpha: (calls > 0 ? 0.9 : 0.5) * fade, ...LINE_STYLE });
      }
      arrow(e, path, color, fade);
    }
    this.publishEdgeAnchors();
  }

  private drawNodes(f: Frame) {
    this.nodes.clear();
    this.nodesDim.clear();
    const seen = new Set<number>();
    const hw = CARD_W / 2;
    const hh = CARD_H / 2;
    const selection = this.selection();
    for (let i = 0; i < f.nodeIndices.length; i++) {
      const o = i * NODE_STRIDE;
      const x = f.nodeData[o] ?? 0;
      const y = f.nodeData[o + 1] ?? 0;
      const status = f.nodeData[o + 3] ?? 0;
      const active = f.nodeData[o + 4] ?? 0;
      const requests = f.nodeData[o + 5] ?? 0;
      const flags = f.nodeData[o + 6] ?? 0;
      const idx = f.nodeIndices[i] ?? 0;
      const statusColor = STATUS_COLORS[status] ?? STATUS_COLORS[0] ?? 0x64748b;
      const external = (flags & FLAG_EXTERNAL) !== 0;
      // A service keeps its own colour unless it is degraded or failing.
      const color = status >= 2 ? statusColor : external ? EXTERNAL_COLOR : (this.colors.get(idx) ?? statusColor);
      const inferred = (flags & FLAG_INFERRED) !== 0;
      const dimmed = (flags & FLAG_DIMMED) !== 0;
      const n = dimmed ? this.nodesDim : this.nodes;
      const focused = nodeFocused(idx, this.hostForNode(idx), selection);
      const fade = focused ? 1 : SELECTION_DIM_ALPHA;
      const left = x - hw;
      const top = y - hh;

      if (status > 0) {
        n.roundRect(left - 3, top - 3, CARD_W + 6, CARD_H + 6, 15).stroke({ color, width: 6, alpha: (status >= 2 ? 0.22 : 0.1) * fade });
      }
      if (external) {
        // Outside the project: dashed border, no solid frame, so it never reads as one of our services.
        n.roundRect(left, top, CARD_W, CARD_H, 4).fill({ color: CARD_BG, alpha: 0.9 * fade });
        for (const dash of dashedRect(left, top, CARD_W, CARD_H)) line(n, dash);
        n.stroke({ color, width: 1.75, alpha: 0.9 * fade, ...LINE_STYLE });
      } else {
        n.roundRect(left, top, CARD_W, CARD_H, 12)
          .fill({ color: CARD_BG, alpha: 0.96 * fade })
          .stroke({ color, width: 1.5, alpha: (inferred ? 0.45 : 0.85) * fade });
      }
      if ((flags & FLAG_HIGHLIGHT) !== 0) n.roundRect(left - 5, top - 5, CARD_W + 10, CARD_H + 10, 16).stroke({ color: ACCENT, width: 2 });
      if (idx === this.selectedIndex) n.roundRect(left - 8, top - 8, CARD_W + 16, CARD_H + 16, 18).stroke({ color: 0xffffff, width: 2, alpha: 0.9 });

      const ix = left + 12;
      const iy = y - 15;
      n.roundRect(ix, iy, 30, 30, 9).fill({ color, alpha: 0.14 });
      const resource = (flags & FLAG_RESOURCE) !== 0;
      drawKindIcon(n, this.kinds[idx], resource, ix + 15, iy + 15, color);
      n.circle(left + CARD_W - 12, top + 12, 3.5).fill({ color: statusColor, alpha: status > 0 ? 1 : 0.6 });

      // Labels live in world space: zooming out shrinks them with their card instead of hiding them.
      seen.add(idx);
      let t = this.textPool.get(idx);
      if (!t) {
        const name = new Text({ text: truncate(this.names[idx] ?? `#${idx}`, 17), style: this.nameStyle });
        const sub = new Text({ text: "", style: this.subStyle });
        t = { name, sub, subValue: "" };
        this.textPool.set(idx, t);
        this.labels.addChild(name, sub);
      }
      const subValue = nodeSubLabel(external, active, requests);
      if (subValue !== t.subValue) {
        t.sub.text = subValue;
        t.subValue = subValue;
      }
      t.name.visible = true;
      t.sub.visible = true;
      t.name.alpha = (dimmed ? DIM_ALPHA + 0.1 : 1) * fade;
      t.sub.alpha = (dimmed ? DIM_ALPHA + 0.1 : 1) * fade;
      t.name.position.set(left + 52, y - 17);
      t.sub.position.set(left + 52, y + 2);
    }
    for (const [idx, t] of this.textPool) {
      if (!seen.has(idx)) {
        t.name.visible = false;
        t.sub.visible = false;
      }
    }
    // Which cards carry a visible label, for tests (node indices).
    this.host?.setAttribute("data-labelled-nodes", Array.from(seen).join(","));
  }

  /**
   * Every dot is one recorded call, coloured by the service where its request started. The
   * request (filled dot) goes caller -> callee over t0..t1, waits at the callee while it works,
   * and the answer (hollow ring) goes back callee -> caller over t2..t3. Since the engine keeps
   * a callee's calls inside its own t1..t2, a dot only leaves a service after one arrived.
   */
  private drawHops() {
    const g = this.hops;
    g.clear();
    const f = this.frame;
    if (!f || f.hopData.length === 0) return;
    const now = animTimeMs(this.clock, this.frameTimeUs, this.frameArrivedAt, performance.now(), performance.timeOrigin);
    const lit = new Set<number>();
    const data = f.hopData;
    for (let o = 0; o + HOP_STRIDE <= data.length; o += HOP_STRIDE) {
      const edge = data[o] ?? 0;
      const geom = this.edgeGeom.get(edge);
      if (!geom) continue;
      const flags = data[o + 5] ?? 0;
      const phase = hopPhase(data[o + 1] ?? 0, data[o + 2] ?? 0, data[o + 3] ?? 0, data[o + 4] ?? 0, now, (flags & HOP_LEAF) !== 0);
      if (phase === null) continue;
      const origin = data[o + 6] ?? -1;
      const color = (origin >= 0 ? this.colors.get(origin) : undefined) ?? ACCENT;
      const error = (flags & HOP_ERROR) !== 0;
      const k = (flags & HOP_DIMMED) !== 0 ? DIM_ALPHA : 1;
      if (!lit.has(edge)) {
        lit.add(edge);
        line(g, geom.path.pts).stroke({ color, width: 3, alpha: 0.3 * k, ...LINE_STYLE });
      }
      const r = (flags & HOP_HIGHLIGHT) !== 0 ? 6.5 : 4.5;
      const [px, py] = pointAt(geom.path, phase.p);
      if (error) g.circle(px, py, r * 2.6).fill({ color: EDGE_ERROR, alpha: 0.28 * k });
      else g.circle(px, py, r * 2.4).fill({ color, alpha: 0.14 * k });
      if (phase.dir === "request") {
        g.circle(px, py, r).fill({ color, alpha: 0.95 * k }).stroke({ color: 0xffffff, width: 1.2, alpha: 0.7 * k });
      } else if (phase.dir === "wait") {
        g.circle(px, py, r * 0.75).fill({ color, alpha: 0.5 * k });
      } else {
        g.circle(px, py, r).fill({ color: CARD_BG, alpha: 0.9 * k }).stroke({ color: error ? EDGE_ERROR : color, width: 2.2, alpha: k });
      }
    }
  }
}

function near(a: number, b: number) {
  return Math.abs(a - b) < 0.5;
}

function truncate(s: string, max: number) {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

/** Adds a polyline (flat x, y pairs) to the current path. */
function line(gr: Graphics, pts: number[]) {
  gr.moveTo(pts[0] ?? 0, pts[1] ?? 0);
  for (let i = 2; i + 1 < pts.length; i += 2) gr.lineTo(pts[i] ?? 0, pts[i + 1] ?? 0);
  return gr;
}

function arrow(gr: Graphics, path: EdgePath, color: number, alpha = 1) {
  const [bx, by] = pointAt(path, 1);
  const [ux, uy] = endDirection(path);
  const size = 8;
  gr.poly([
    bx,
    by,
    bx - ux * size - uy * size * 0.55,
    by - uy * size + ux * size * 0.55,
    bx - ux * size + uy * size * 0.55,
    by - uy * size - ux * size * 0.55,
  ]).fill({ color, alpha });
}

function drawKindIcon(g: Graphics, kind: EntityKindDto | undefined, resource: boolean, cx: number, cy: number, color: number) {
  if (kind === "external_dependency" || kind === "external_client") {
    // Globe; an external client also gets an arrow pointing in.
    g.circle(cx, cy, 8.5).stroke({ color, width: 1.5 });
    g.ellipse(cx, cy, 3.6, 8.5).stroke({ color, width: 1.1, alpha: 0.8 });
    g.moveTo(cx - 8.5, cy).lineTo(cx + 8.5, cy).stroke({ color, width: 1.1, alpha: 0.8 });
    if (kind === "external_client") g.circle(cx + 8, cy - 8, 2.5).fill({ color });
    return;
  }
  if (kind === "database" || (resource && kind === undefined)) {
    const w = 8;
    g.ellipse(cx, cy - 5, w, 2.8).stroke({ color, width: 1.5 });
    g.moveTo(cx - w, cy - 5).lineTo(cx - w, cy + 5).stroke({ color, width: 1.5 });
    g.moveTo(cx + w, cy - 5).lineTo(cx + w, cy + 5).stroke({ color, width: 1.5 });
    g.moveTo(cx - w, cy + 5).quadraticCurveTo(cx, cy + 10, cx + w, cy + 5).stroke({ color, width: 1.5 });
    return;
  }
  if (kind === "queue" || kind === "cache" || resource) {
    g.roundRect(cx - 8, cy - 8, 16, 16, 3).stroke({ color, width: 1.5 });
    g.moveTo(cx - 8, cy).lineTo(cx + 8, cy).stroke({ color, width: 1.2, alpha: 0.7 });
    return;
  }
  const pts: number[] = [];
  for (let k = 0; k < 6; k++) {
    const a = Math.PI / 6 + (k * Math.PI) / 3;
    pts.push(cx + Math.cos(a) * 9, cy + Math.sin(a) * 9);
  }
  g.poly(pts).stroke({ color, width: 1.6 });
  g.circle(cx, cy, 2.5).fill({ color });
}
