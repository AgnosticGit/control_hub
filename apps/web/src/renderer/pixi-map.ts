// PixiJS (WebGL) renderer for the service map. It only draws: positions,
// statuses and flags come precomputed from the engine frame buffers. Pan and
// zoom are local; the resulting world-space viewport is reported back so the
// engine can cull and choose a level of detail.

// Replaces Pixi's `new Function` code generation so the page runs without CSP 'unsafe-eval'.
import "pixi.js/unsafe-eval";
import { Application, Container, Graphics, Text, TextStyle } from "pixi.js";
import {
  EDGE_STRIDE,
  FLAG_HIGHLIGHT,
  FLAG_INFERRED,
  FLAG_PROBLEM,
  FLAG_RESOURCE,
  NODE_STRIDE,
  type Frame,
} from "@controlhub/web-engine";

export interface MapCallbacks {
  onViewport(x: number, y: number, width: number, height: number, scale: number): void;
  onPick(worldX: number, worldY: number): void;
}

const STATUS_COLORS = [0x64748b, 0x22d3ee, 0xf59e0b, 0xef4444];
const EDGE_OK = 0x3b82f6;
const EDGE_INFERRED = 0x64748b;
const EDGE_ERROR = 0xef4444;
const EDGE_WARN = 0xf59e0b;
const ACCENT = 0x22d3ee;
const MIN_SCALE = 0.15;
const MAX_SCALE = 4;

export class PixiMap {
  private app: Application | null = null;
  private readonly world = new Container();
  private readonly edges = new Graphics();
  private readonly pulses = new Graphics();
  private readonly nodes = new Graphics();
  private readonly labels = new Container();
  private readonly labelPool = new Map<number, Text>();
  private readonly labelStyle = new TextStyle({ fill: 0xe5ebf5, fontSize: 12, fontFamily: "system-ui, sans-serif" });
  private frame: Frame | null = null;
  private names: string[] = [];
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
      background: 0x0b1017,
      preference: "webgl",
      autoDensity: true,
      resolution: Math.min(window.devicePixelRatio || 1, 2),
    });
    if (this.destroyed) {
      app.destroy(true);
      return;
    }
    this.app = app;
    app.canvas.setAttribute("aria-hidden", "true");
    host.appendChild(app.canvas);
    this.world.addChild(this.edges, this.pulses, this.nodes, this.labels);
    app.stage.addChild(this.world);
    this.reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    app.ticker.add(() => this.drawPulses());
    this.bindInput(app.canvas);
    const ro = new ResizeObserver(() => this.reportViewport());
    ro.observe(host);
    this.cleanup.push(() => ro.disconnect());
  }

  setNames(names: string[]) {
    if (names.length === this.names.length && names.every((n, i) => n === this.names[i])) return;
    this.names = names;
    for (const t of this.labelPool.values()) t.destroy();
    this.labelPool.clear();
    this.labels.removeChildren();
  }

  setSelected(index: number | null) {
    this.selectedIndex = index;
    if (this.frame) this.draw(this.frame);
  }

  render(frame: Frame) {
    this.frame = frame;
    if (!this.app) return;
    if (!this.fitted && frame.nodeIndices.length > 0) {
      this.fitted = true;
      this.fit();
    }
    this.draw(frame);
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
    const pad = 120;
    const w = app.screen.width;
    const h = app.screen.height;
    const scale = Math.max(
      MIN_SCALE,
      Math.min(1.5, Math.min(w / (maxX - minX + pad * 2), h / (maxY - minY + pad * 2))),
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

  private reportViewport() {
    const app = this.app;
    if (!app) return;
    const s = this.world.scale.x;
    this.callbacks.onViewport(-this.world.x / s, -this.world.y / s, app.screen.width / s, app.screen.height / s, s);
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
    };
    const up = (e: PointerEvent) => {
      if (!dragging) return;
      dragging = false;
      canvas.releasePointerCapture(e.pointerId);
      if (moved < 4) {
        const rect = canvas.getBoundingClientRect();
        const s = this.world.scale.x;
        this.callbacks.onPick((e.clientX - rect.left - this.world.x) / s, (e.clientY - rect.top - this.world.y) / s);
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

  private draw(f: Frame) {
    const e = this.edges;
    e.clear();
    for (let i = 0; i < f.edgeIndices.length; i++) {
      const o = i * EDGE_STRIDE;
      const x1 = f.edgeData[o] ?? 0;
      const y1 = f.edgeData[o + 1] ?? 0;
      const x2 = f.edgeData[o + 2] ?? 0;
      const y2 = f.edgeData[o + 3] ?? 0;
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
          : inferred
            ? EDGE_INFERRED
            : EDGE_OK;
      const width = highlight ? 3 : 1.5 + Math.min(3, Math.log10(1 + calls));
      if (inferred) this.dashed(e, x1, y1, x2, y2, color, width);
      else e.moveTo(x1, y1).lineTo(x2, y2).stroke({ color, width, alpha: 0.85 });
      this.arrow(e, x1, y1, x2, y2, color);
    }

    const n = this.nodes;
    n.clear();
    const lod = f.summary.lod;
    const seen = new Set<number>();
    for (let i = 0; i < f.nodeIndices.length; i++) {
      const o = i * NODE_STRIDE;
      const x = f.nodeData[o] ?? 0;
      const y = f.nodeData[o + 1] ?? 0;
      const r = f.nodeData[o + 2] ?? 20;
      const status = f.nodeData[o + 3] ?? 0;
      const active = f.nodeData[o + 4] ?? 0;
      const flags = f.nodeData[o + 6] ?? 0;
      const idx = f.nodeIndices[i] ?? 0;
      const color = STATUS_COLORS[status] ?? STATUS_COLORS[0] ?? 0x64748b;
      if ((flags & FLAG_RESOURCE) !== 0) {
        n.roundRect(x - r, y - r * 0.75, r * 2, r * 1.5, 6).fill({ color: 0x111827 }).stroke({ color, width: 2 });
      } else {
        n.circle(x, y, r).fill({ color: 0x111827 }).stroke({ color, width: 2.5 + Math.min(4, active) });
      }
      if ((flags & FLAG_INFERRED) !== 0) n.circle(x, y, r + 5).stroke({ color: EDGE_INFERRED, width: 1, alpha: 0.6 });
      if ((flags & FLAG_HIGHLIGHT) !== 0) n.circle(x, y, r + 8).stroke({ color: ACCENT, width: 2 });
      if (idx === this.selectedIndex) n.circle(x, y, r + 11).stroke({ color: 0xffffff, width: 2 });

      if (lod >= 1) {
        seen.add(idx);
        let label = this.labelPool.get(idx);
        if (!label) {
          label = new Text({ text: this.names[idx] ?? `#${idx}`, style: this.labelStyle });
          label.anchor.set(0.5, 0);
          this.labelPool.set(idx, label);
          this.labels.addChild(label);
        }
        label.visible = true;
        label.position.set(x, y + r + 6);
      }
    }
    for (const [idx, label] of this.labelPool) {
      if (!seen.has(idx)) label.visible = false;
    }
  }

  private drawPulses() {
    const g = this.pulses;
    g.clear();
    const f = this.frame;
    if (!f || this.reducedMotion) return;
    const phase = (performance.now() % 1600) / 1600;
    const limit = Math.min(f.edgeIndices.length, 500);
    for (let i = 0; i < limit; i++) {
      const o = i * EDGE_STRIDE;
      const active = f.edgeData[o + 4] ?? 0;
      const calls = f.edgeData[o + 5] ?? 0;
      if (active <= 0 && calls <= 0) continue;
      const x1 = f.edgeData[o] ?? 0;
      const y1 = f.edgeData[o + 1] ?? 0;
      const x2 = f.edgeData[o + 2] ?? 0;
      const y2 = f.edgeData[o + 3] ?? 0;
      const flags = f.edgeData[o + 7] ?? 0;
      const color = (flags & FLAG_PROBLEM) !== 0 ? EDGE_ERROR : ACCENT;
      g.circle(x1 + (x2 - x1) * phase, y1 + (y2 - y1) * phase, 3).fill({ color, alpha: 0.9 });
    }
  }

  private dashed(g: Graphics, x1: number, y1: number, x2: number, y2: number, color: number, width: number) {
    const len = Math.hypot(x2 - x1, y2 - y1);
    const dash = 8;
    const steps = Math.floor(len / (dash * 2));
    const dx = (x2 - x1) / len;
    const dy = (y2 - y1) / len;
    for (let s = 0; s < steps; s++) {
      const a = s * dash * 2;
      g.moveTo(x1 + dx * a, y1 + dy * a).lineTo(x1 + dx * (a + dash), y1 + dy * (a + dash));
    }
    g.stroke({ color, width, alpha: 0.8 });
  }

  private arrow(g: Graphics, x1: number, y1: number, x2: number, y2: number, color: number) {
    const len = Math.hypot(x2 - x1, y2 - y1);
    if (len < 60) return;
    const ux = (x2 - x1) / len;
    const uy = (y2 - y1) / len;
    const tipX = x2 - ux * 32;
    const tipY = y2 - uy * 32;
    const size = 7;
    g.poly([
      tipX,
      tipY,
      tipX - ux * size - uy * size * 0.6,
      tipY - uy * size + ux * size * 0.6,
      tipX - ux * size + uy * size * 0.6,
      tipY - uy * size - ux * size * 0.6,
    ]).fill({ color });
  }
}
