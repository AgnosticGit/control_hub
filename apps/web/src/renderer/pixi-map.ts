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
import { CHIP_H, CHIP_W, FRAME_HEADER, NOISY_LINK, badgeText, chipAt, frameAt, frameColor, linkLabelAt } from "./infra-layer";
import { DEFAULT_EDGE_STYLE, build, dashes, edgePath, endDirection, pointAt, type EdgePath, type EdgeStyle } from "./edge-path";
import { routeEdges, type RouteEdge, type RouteNode } from "./edge-route";
import { dashedRect, nodeSubLabel } from "./external";
import { ACCENT, CARD_BG, EDGE_ERROR, EDGE_INFERRED, EDGE_OK, EDGE_WARN, EXTERNAL_COLOR, STATUS_COLORS } from "./map-colors";
import { cullRect } from "./cull";
import { gridLines } from "./grid";
import { animTimeMs, hopPhase, type AnimClock } from "./hops";
import { serviceColors } from "./service-colors";

/** Pan (screen px) and zoom of the map. */
export interface MapCamera {
  x: number;
  y: number;
  scale: number;
}

export interface MapCallbacks {
  onViewport(x: number, y: number, width: number, height: number, scale: number): void;
  onPick(worldX: number, worldY: number): void;
  /** A click on a machine frame (infrastructure layer) outside any card or chip. */
  onPickHost?(hostId: string): void;
}

const MIN_SCALE = 0.15;
const MAX_SCALE = 4;
/** Node card size in world units (layout spacing is 300 × 140). */
const CARD_W = 172;
const CARD_H = 56;
const CARD_BOX = { hw: CARD_W / 2 + 4, hh: CARD_H / 2 + 4 };
const ROUTE_BOX = { ...CARD_BOX, portSpan: CARD_H - 20 };
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
  private readonly labels = new Container();
  private readonly textPool = new Map<number, NodeText>();
  /** Infrastructure layer: frames and chips under the cards, links and badges above them. */
  private readonly infraBack = new Graphics();
  private readonly infraFront = new Graphics();
  private readonly infraLabels = new Container();
  private infraView: InfraView | null = null;
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
  private routes = new Map<number, Route>();
  private names: string[] = [];
  private kinds: EntityKindDto[] = [];
  private colors = new Map<number, number>();
  private selectedIndex: number | null = null;
  private fitted = false;
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
      this.infraFront,
      this.hops,
      this.labels,
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
    this.reroute();
    const infra = topology?.infra ?? null;
    // Switching the layer moves every card: fit again on the next frame.
    const refit = (infra === null) !== (this.infraView === null);
    this.infraView = infra;
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
    this.selectedIndex = index;
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
    const padX = CARD_W;
    const padY = CARD_H * 2;
    const w = app.screen.width;
    const h = app.screen.height;
    const scale = Math.max(
      MIN_SCALE,
      Math.min(1.3, Math.min(w / (maxX - minX + padX * 2), h / (maxY - minY + padY * 2))),
    );
    this.world.scale.set(scale);
    this.world.position.set(w / 2 - ((minX + maxX) / 2) * scale, h / 2 - ((minY + maxY) / 2) * scale);
    this.reportViewport();
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
    const app = this.app;
    if (!app) return;
    const s = this.world.scale.x;
    const r = cullRect({ x: -this.world.x / s, y: -this.world.y / s, width: app.screen.width / s, height: app.screen.height / s }, CARD_W / 2 + 8);
    this.callbacks.onViewport(r.x, r.y, r.width, r.height, s);
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
      this.callbacks.onPick(chip.x, chip.y);
      return;
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
    if (!view) return;
    for (const f of view.frames) {
      const color = frameColor(f);
      back.roundRect(f.x, f.y, f.width, f.height, 18).fill({ color, alpha: 0.05 }).stroke({ color, width: 1.5, alpha: f.hot ? 0.9 : 0.55 });
      back.roundRect(f.x, f.y, f.width, FRAME_HEADER - 8, 18).fill({ color, alpha: 0.1 });
      const label = new Text({ text: f.label, style: f.hot ? this.frameHotStyle : this.frameStyle });
      label.position.set(f.x + 16, f.y + 10);
      this.infraLabels.addChild(label);
    }
    for (const c of view.chips) {
      const color = this.colors.get(c.node) ?? 0x64748b;
      back.roundRect(c.x - CHIP_W / 2, c.y - CHIP_H / 2, CHIP_W, CHIP_H, 8).fill({ color: CARD_BG, alpha: 0.95 }).stroke({ color, width: 1.25, alpha: 0.8 });
      back.circle(c.x - CHIP_W / 2 + 12, c.y, 3.5).fill({ color });
      const t = new Text({ text: truncate(c.label, 20), style: this.chipStyle });
      t.position.set(c.x - CHIP_W / 2 + 22, c.y - 7);
      this.infraLabels.addChild(t);
    }
    const pos = new Map<number, RouteNode>();
    for (const n of this.routeNodes) pos.set(n.index, n);
    for (const b of view.badges) {
      const n = pos.get(b.node);
      if (!n) continue;
      const text = badgeText(b.count, b.hosts);
      const t = new Text({ text, style: this.chipStyle });
      const w = t.width + 14;
      const x = n.x + CARD_W / 2 - w + 6;
      const y = n.y - CARD_H / 2 - 11;
      front.roundRect(x, y, w, 20, 10).fill({ color: 0x4c1d95, alpha: 0.95 }).stroke({ color: 0xa78bfa, width: 1 });
      t.position.set(x + 7, y + 3);
      this.infraLabels.addChild(t);
    }
    for (const l of view.links) {
      for (const dash of dashes(build([l.from[0], l.from[1], l.to[0], l.to[1]]), 8, 6)) line(front, dash);
      front.stroke({ color: NOISY_LINK, width: 3, alpha: 0.95, ...LINE_STYLE });
      front.circle(l.to[0], l.to[1], 6).stroke({ color: NOISY_LINK, width: 2 });
      const [mx, my] = linkLabelAt(l.from, l.to);
      const t = new Text({ text: l.label, style: this.linkStyle });
      front.roundRect(mx - t.width / 2 - 6, my - 10, t.width + 12, 20, 6).fill({ color: CARD_BG, alpha: 0.95 }).stroke({ color: NOISY_LINK, width: 1 });
      t.position.set(mx - t.width / 2, my - 8);
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
  }

  private drawEdges(f: Frame) {
    this.edges.clear();
    this.edgesDim.clear();
    this.edgeGeom.clear();
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
      const route = this.routes.get(index);
      // A route is only valid for the positions it was computed for (topology may lag a frame).
      const fresh = route && near(route.sx, sx) && near(route.sy, sy) && near(route.tx, tx) && near(route.ty, ty);
      const path = fresh ? route.path : edgePath(this.edgeStyle, sx, sy, tx, ty, CARD_BOX);
      if (!path) continue;
      this.edgeGeom.set(index, { path, color });
      const e = (flags & FLAG_DIMMED) !== 0 ? this.edgesDim : this.edges;
      const width = highlight ? 2.5 : 1.25 + Math.min(1.5, Math.log10(1 + calls) * 0.6);
      if (inferred) {
        for (const dash of dashes(path, 6, 5)) line(e, dash);
        e.stroke({ color, width, alpha: 0.8, ...LINE_STYLE });
      } else {
        line(e, path.pts).stroke({ color, width: width + 6, alpha: calls > 0 ? 0.1 : 0.05, ...LINE_STYLE });
        line(e, path.pts).stroke({ color, width, alpha: calls > 0 ? 0.9 : 0.5, ...LINE_STYLE });
      }
      arrow(e, path, color);
    }
  }

  private drawNodes(f: Frame) {
    this.nodes.clear();
    this.nodesDim.clear();
    const lod = f.summary.lod;
    const seen = new Set<number>();
    const hw = CARD_W / 2;
    const hh = CARD_H / 2;
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
      const left = x - hw;
      const top = y - hh;

      if (status > 0) {
        n.roundRect(left - 3, top - 3, CARD_W + 6, CARD_H + 6, 15).stroke({ color, width: 6, alpha: status >= 2 ? 0.22 : 0.1 });
      }
      if (external) {
        // Outside the project: dashed border, no solid frame, so it never reads as one of our services.
        n.roundRect(left, top, CARD_W, CARD_H, 4).fill({ color: CARD_BG, alpha: 0.9 });
        for (const dash of dashedRect(left, top, CARD_W, CARD_H)) line(n, dash);
        n.stroke({ color, width: 1.75, alpha: 0.9, ...LINE_STYLE });
      } else {
        n.roundRect(left, top, CARD_W, CARD_H, 12)
          .fill({ color: CARD_BG, alpha: 0.96 })
          .stroke({ color, width: 1.5, alpha: inferred ? 0.45 : 0.85 });
      }
      if ((flags & FLAG_HIGHLIGHT) !== 0) n.roundRect(left - 5, top - 5, CARD_W + 10, CARD_H + 10, 16).stroke({ color: ACCENT, width: 2 });
      if (idx === this.selectedIndex) n.roundRect(left - 8, top - 8, CARD_W + 16, CARD_H + 16, 18).stroke({ color: 0xffffff, width: 2, alpha: 0.9 });

      const ix = left + 12;
      const iy = y - 15;
      n.roundRect(ix, iy, 30, 30, 9).fill({ color, alpha: 0.14 });
      const resource = (flags & FLAG_RESOURCE) !== 0;
      drawKindIcon(n, this.kinds[idx], resource, ix + 15, iy + 15, color);
      n.circle(left + CARD_W - 12, top + 12, 3.5).fill({ color: statusColor, alpha: status > 0 ? 1 : 0.6 });

      if (lod >= 1) {
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
        t.name.alpha = dimmed ? DIM_ALPHA + 0.1 : 1;
        t.sub.alpha = dimmed ? DIM_ALPHA + 0.1 : 1;
        t.name.position.set(left + 52, y - 17);
        t.sub.position.set(left + 52, y + 2);
      }
    }
    for (const [idx, t] of this.textPool) {
      if (!seen.has(idx)) {
        t.name.visible = false;
        t.sub.visible = false;
      }
    }
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

function arrow(gr: Graphics, path: EdgePath, color: number) {
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
  ]).fill({ color });
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
