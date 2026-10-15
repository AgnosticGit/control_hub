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
  type InfrastructureSnapshot,
  type InfraView,
  type TopologyView,
} from "@karbird/web-engine";
import { FRAME_HEADER, NOISY_LINK, frameAt, frameHeaderAt, frameLook, linkArc, linkLabelAt } from "./infra-layer";
import { DEFAULT_EDGE_STYLE, build, zoomInvariantDashes, endDirection, pointAt, type EdgePath, type EdgeStyle } from "./edge-path";
import { edgeAt, midpoint } from "./edge-hit";
import { routeEdges, type RouteEdge, type RouteNode } from "./edge-route";
import { edgeCardGroups } from "./replica-edge-pairs";
import { edgeBranchPaths, routeBetweenCards } from "./replica-edge-paths";
import { serviceInstanceCounts } from "./replica-counts";
import { dashedRect, nodeSubLabel } from "./external";
import { ACCENT, CARD_BG, EDGE_ERROR, EDGE_INFERRED, EDGE_OK, EDGE_WARN, EXTERNAL_COLOR, STATUS_COLORS } from "./map-colors";
import { cullRect } from "./cull";
import { fitCamera, MIN_MAP_SCALE, overlayInset, zoomCamera, zoomOutFloorScale } from "./fit";
import { gridLines } from "./grid";
import { animTimeMs, hopPhase, type AnimClock } from "./hops";
import { serviceColors } from "./service-colors";
import { cardGeometry, COMPACT_CARD_SIZE, EXPANDED_CARD_SIZE, routeBoxForCard, serviceMetricCardSize } from "./map-card-geometry";
import { displayMapPoint, displayTopologyForMetrics, mapDisplayScale, sourceMapPoint } from "./map-display-layout";
import { deriveServiceCardMetrics, formatBytes, formatBytesPerSecond, type ServiceCardMetrics } from "@/components/workspace/service-card-metrics";
import { serviceMetricPresentation } from "@/components/workspace/service-card-presentation";
import { SERVICE_CARD_LAYOUT } from "./service-card-layout";
import { metricBar, METRIC_BAR_COLORS } from "./service-metric-bars";
import { edgeFadeAlpha, frameFocused, incidentEdgeIndices, incidentGlowWidth, nodeFocused, SELECTION_DIM_ALPHA, selectionAlpha, type MapSelection } from "./selection-style";

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
const EDGE_SELECTED = 0x19d4ee;

const MIN_SCALE = MIN_MAP_SCALE;
const MAX_SCALE = 4;
/** Backend layout reserves the expanded bounds even while cards are compact. */
const MAX_CARD_SIZE = EXPANDED_CARD_SIZE;
const LINE_STYLE = { join: "round", cap: "round" } as const;
/** Opacity of switched-off services and everything attached to them. */
const DIM_ALPHA = 0.22;
const GRID_COLOR = 0x6284a7;
const GRID_MINOR_ALPHA = 0.06;
const GRID_MAJOR_ALPHA = 0.12;

interface EdgeGeom {
  path: EdgePath;
  paths: EdgePath[];
  trunks: EdgePath[];
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

interface MetricCardText {
  cpuLabel: Text;
  cpuValue: Text;
  memoryLabel: Text;
  memoryValue: Text;
  receive: Text;
  transmit: Text;
  incoming: Text;
  outgoing: Text;
}

export class PixiMap {
  private app: Application | null = null;
  private latestTopologyRevision = -1;
  private host: HTMLElement | null = null;
  private searchLayer: HTMLDivElement | null = null;
  private searchWorld: HTMLDivElement | null = null;
  private readonly searchLabels = new Map<number, HTMLDivElement>();
  private readonly replicaSearchLabels = new Map<string, HTMLDivElement>();
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
  private readonly countTextPool = new Map<number, Text>();
  private readonly metricTextPool = new Map<string, MetricCardText>();
  private readonly metricCache = new Map<string, ServiceCardMetrics>();
  private metricCacheFrame: Frame | null = null;
  private instanceCounts = new Map<string, number>();
  /** Infrastructure layer: machine frames below the cards, links and badges above them. */
  private readonly infraBack = new Graphics();
  private readonly infraFront = new Graphics();
  private readonly infraLabels = new Container();
  private infraView: InfraView | null = null;
  private serviceMetricSnapshot: InfrastructureSnapshot | null = null;
  private serviceMetricsEnabled = false;
  private serviceMetricsLoading = false;
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
  private readonly metricStyle = new TextStyle({ fill: 0xcbd5e1, fontSize: 9, fontFamily: "'JetBrains Mono', ui-monospace, monospace" });
  private frame: Frame | null = null;
  private frameTimeUs = 0;
  private frameArrivedAt = 0;
  private clock: AnimClock = { kind: "frame", rate: 0 };
  private edgeGeom = new Map<number, EdgeGeom>();
  private edgeStyle: EdgeStyle = DEFAULT_EDGE_STYLE;
  private routeNodes: RouteNode[] = [];
  private routeEdges: RouteEdge[] = [];
  private topology: TopologyView | null = null;
  private readonly selectedIncidentEdges = new Set<number>();
  private readonly hostByNode = new Map<number, string>();
  private routes = new Map<number, Route>();
  private names: string[] = [];
  private ids: string[] = [];
  private kinds: EntityKindDto[] = [];
  private colors = new Map<number, number>();
  private selectedIndex: number | null = null;
  private selectedEdge: number | null = null;
  private selectedEdgeNodes: readonly [number, number] | null = null;
  private edgeAnchors = "";
  private fitted = false;
  private minimumCamera: MapCamera = { x: 0, y: 0, scale: MIN_SCALE };
  private pendingCamera: MapCamera | null = null;
  /** The camera is where `fit` put it (no pan or zoom since). */
  private cameraIsFit = false;
  /** Screen pixels occupied by the Machines overlay; fit keeps cards below it. */
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
    const searchLayer = document.createElement("div");
    Object.assign(searchLayer.style, {
      position: "absolute",
      inset: "0",
      overflow: "hidden",
      pointerEvents: "none",
      userSelect: "none",
      zIndex: "1",
    });
    searchLayer.setAttribute("role", "region");
    searchLayer.setAttribute("aria-label", "System Map text");
    const searchWorld = document.createElement("div");
    Object.assign(searchWorld.style, {
      position: "absolute",
      left: "0",
      top: "0",
      transformOrigin: "top left",
      pointerEvents: "none",
      userSelect: "none",
    });
    searchLayer.appendChild(searchWorld);
    host.appendChild(searchLayer);
    this.searchLayer = searchLayer;
    this.searchWorld = searchWorld;
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
    const ro = new ResizeObserver(() => {
      const width = host.clientWidth;
      const height = host.clientHeight;
      if (width > 0 && height > 0 && (app.screen.width !== width || app.screen.height !== height)) {
        app.renderer.resize(width, height);
      }
      if (this.cameraIsFit) this.fit();
      else if (this.frame?.nodeIndices.length) this.refreshZoomFloor();
      else this.reportViewport();
    });
    ro.observe(host);
    this.cleanup.push(() => ro.disconnect());
  }

  setTopology(topology: TopologyView | null) {
    if (topology && topology.revision < this.latestTopologyRevision) return;
    if (topology) this.latestTopologyRevision = topology.revision;
    const displayTopology = topology ? displayTopologyForMetrics(topology, this.serviceMetricsEnabled) : null;
    this.host?.setAttribute("data-layout-nodes", JSON.stringify(displayTopology?.nodes.map(n=>({id:n.id,name:n.name,x:n.x,y:n.y})) ?? []));
    const nodes = displayTopology?.nodes ?? [];
    const names: string[] = [];
    const ids: string[] = [];
    const kinds: EntityKindDto[] = [];
    for (const n of nodes) {
      names[n.index] = n.name;
      ids[n.index] = n.id;
      kinds[n.index] = n.kind;
    }
    this.topology = topology;
    this.ids = ids;
    this.kinds = kinds;
    this.colors = serviceColors(nodes);
    const nextNodes = nodes.map((n) => ({ index: n.index, x: n.x, y: n.y }));
    const nextEdges = (displayTopology?.edges ?? []).map((e) => ({ index: e.index, source: e.source, target: e.target, points: e.points }));
    const reroute = nextNodes.length !== this.routeNodes.length || nextEdges.length !== this.routeEdges.length
      || nextNodes.some((n,i)=>{const old=this.routeNodes[i];return !old || n.index!==old.index || n.x!==old.x || n.y!==old.y;})
      || nextEdges.some((e,i)=>{const old=this.routeEdges[i];return !old || e.index!==old.index || e.source!==old.source || e.target!==old.target || e.points.length!==old.points.length || e.points.some((p,j)=>p[0]!==old.points[j]?.[0] || p[1]!==old.points[j]?.[1]);});
    this.routeNodes = nextNodes;
    this.routeEdges = nextEdges;
    this.refreshSelectedIncidentEdges();
    const selectedRoute = this.routeEdges.find((edge) => edge.index === this.selectedEdge);
    this.selectedEdgeNodes = selectedRoute ? [selectedRoute.source, selectedRoute.target] : null;
    if (reroute) this.reroute();
    const infra = displayTopology?.infra ?? null;
    // Server revisions change geometry without resetting the user's camera.
    this.infraView = infra;
    if (this.frame?.nodeIndices.length) {
      if (this.cameraIsFit) this.fit();
      else this.refreshZoomFloor();
    }
    this.hostByNode.clear();
    for (const node of this.routeNodes) {
      const frame = infra?.frames.find((candidate) => node.x >= candidate.x && node.x <= candidate.x + candidate.width && node.y >= candidate.y && node.y <= candidate.y + candidate.height);
      if (frame) this.hostByNode.set(node.index, frame.host_id);
    }
    this.drawInfra();
    this.syncSearchNodes(nodes);
    if (names.length !== this.names.length || names.some((n, i) => n !== this.names[i])) {
      this.names = names;
      for (const t of this.textPool.values()) {
        t.name.destroy();
        t.sub.destroy();
      }
      this.textPool.clear();
      for (const text of this.countTextPool.values()) text.destroy();
      this.countTextPool.clear();
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
    this.refreshSelectedIncidentEdges();
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
    } else if (this.pendingCamera && frame.nodeIndices.length > 0) {
      const saved = this.pendingCamera;
      this.pendingCamera = null;
      this.fit();
      this.setCamera(saved);
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
    const displayScale = mapDisplayScale(this.serviceMetricsEnabled);
    for (let i = 0; i < f.nodeIndices.length; i++) {
      const [x, y] = displayMapPoint(f.nodeData[i * NODE_STRIDE] ?? 0, f.nodeData[i * NODE_STRIDE + 1] ?? 0, displayScale);
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x);
      maxY = Math.max(maxY, y);
    }
    const visibleEdges = new Set(f.edgeIndices);
    for (const e of this.routeEdges) {
      if (!visibleEdges.has(e.index)) continue;
      for (const [x, y] of e.points) {
        minX = Math.min(minX, x);
        maxX = Math.max(maxX, x);
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
        padX: this.serviceMetricsEnabled ? MAX_CARD_SIZE.width / 2 : COMPACT_CARD_SIZE.width / 2,
        padY: (this.serviceMetricsEnabled ? MAX_CARD_SIZE.height : COMPACT_CARD_SIZE.height) * 2,
        topInset: this.topInset,
        insetGap: this.serviceMetricsEnabled ? MAX_CARD_SIZE.height : COMPACT_CARD_SIZE.height,
        minScale: MIN_SCALE,
        maxScale: 1.3,
      },
    );
    this.minimumCamera = cam;
    this.world.scale.set(cam.scale);
    this.world.position.set(cam.x, cam.y);
    this.reportViewport();
    this.cameraIsFit = true;
    // Screen y of the top card edge after a fit, for tests: it must stay below any overlay.
    const cardHeight = this.serviceMetricsEnabled ? MAX_CARD_SIZE.height : COMPACT_CARD_SIZE.height;
    this.host?.setAttribute("data-fit-top", String(cam.y + (minY - cardHeight / 2) * cam.scale));
  }

  /** Highlights the frame of the machine open in the inspector. */
  setSelectedHost(hostId: string | null) {
    if (hostId === this.selectedHost) return;
    this.selectedHost = hostId;
    this.syncSelectionFade();
    this.drawInfra();
  }

  setServiceMetricsEnabled(enabled: boolean) {
    if (enabled === this.serviceMetricsEnabled) return;
    this.serviceMetricsEnabled = enabled;
    this.metricCache.clear();
    this.setTopology(this.topology);
  }

  setServiceMetricsLoading(loading: boolean) {
    if (loading === this.serviceMetricsLoading) return;
    this.serviceMetricsLoading = loading;
    this.refreshCardGeometry();
  }

  /** Keeps fit clear of the Machines overlay and refits only while the camera is fitted. */
  setTopInset(px: number) {
    const next = overlayInset(px);
    if (next === this.topInset) return;
    this.topInset = next;
    if (this.cameraIsFit) this.fit();
    else if (this.frame?.nodeIndices.length) this.refreshZoomFloor();
  }

  setServiceMetricSnapshot(snapshot: InfrastructureSnapshot | null) {
    if (snapshot === this.serviceMetricSnapshot) return;
    this.serviceMetricSnapshot = snapshot;
    this.instanceCounts = serviceInstanceCounts(snapshot);
    this.metricCache.clear();
    this.refreshCardGeometry();
  }

  private refreshCardGeometry() {
    if (this.frame) {
      if (this.cameraIsFit) this.fit();
      else this.refreshZoomFloor();
      this.draw(this.frame);
      this.drawHops();
    }
  }

  private cardSizeForNode(node: number) {
    if (!this.serviceMetricsEnabled || this.kinds[node] !== "service") return cardGeometry(false);
    const metrics = this.frame ? this.metricsForService(this.ids[node] ?? "", this.frame) : null;
    const presentation = serviceMetricPresentation(metrics, this.serviceMetricsLoading);
    return serviceMetricCardSize([...presentation.rows, ...presentation.loadingRows].map((row) =>
      row === "receive" || row === "transmit" ? "network" : row === "incoming" || row === "outgoing" ? "requests" : row,
    ));
  }

  private routeBoxForNode(node: number) {
    return routeBoxForCard(this.cardSizeForNode(node));
  }

  private metricsForService(serviceId: string, frame: Frame): ServiceCardMetrics | null {
    if (!this.serviceMetricsEnabled || !this.serviceMetricSnapshot || !this.topology) return null;
    if (this.metricCacheFrame !== frame) {
      this.metricCache.clear();
      this.metricCacheFrame = frame;
    }
    const cached = this.metricCache.get(serviceId);
    if (cached) return cached;
    const metrics = deriveServiceCardMetrics(serviceId, this.topology, frame, this.serviceMetricSnapshot);
    this.metricCache.set(serviceId, metrics);
    return metrics;
  }

  zoomBy(factor: number, sx?: number, sy?: number) {
    const app = this.app;
    if (!app) return;
    const cx = sx ?? app.screen.width / 2;
    const cy = sy ?? app.screen.height / 2;
    const next = zoomCamera(this.getCamera(), factor, cx, cy, this.minimumCamera, MAX_SCALE);
    this.world.scale.set(next.scale);
    this.world.position.set(next.x, next.y);
    this.reportViewport();
  }

  getCamera(): MapCamera {
    return { x: this.world.x, y: this.world.y, scale: this.world.scale.x };
  }

  /** Restores a saved pan and zoom; the map then no longer fits itself to the first frame. */
  setCamera(camera: MapCamera) {
    this.fitted = true;
    if (!this.frame?.nodeIndices.length) {
      this.pendingCamera = camera;
      this.world.scale.set(Math.max(MIN_SCALE, Math.min(MAX_SCALE, camera.scale)));
      this.world.position.set(camera.x, camera.y);
      return;
    }
    const floor = zoomOutFloorScale(this.minimumCamera.scale);
    const scale = Math.max(floor, Math.min(MAX_SCALE, camera.scale));
    const app = this.app;
    const centerX = app?.screen.width ? app.screen.width / 2 : 0;
    const centerY = app?.screen.height ? app.screen.height / 2 : 0;
    const next = camera.scale < floor && app
      ? {
          scale,
          x: centerX - ((centerX - camera.x) / camera.scale) * scale,
          y: centerY - ((centerY - camera.y) / camera.scale) * scale,
        }
      : { ...camera, scale };
    this.world.scale.set(next.scale);
    this.world.position.set(next.x, next.y);
    this.reportViewport();
  }

  panBy(dx: number, dy: number) {
    this.world.position.set(this.world.x + dx, this.world.y + dy);
    this.reportViewport();
  }

  private refreshZoomFloor() {
    const current = this.getCamera();
    const app = this.app;
    this.fit();
    if (!app) return;
    const scale = Math.max(zoomOutFloorScale(this.minimumCamera.scale), Math.min(MAX_SCALE, current.scale));
    const centerX = app.screen.width / 2;
    const centerY = app.screen.height / 2;
    const worldX = (centerX - current.x) / current.scale;
    const worldY = (centerY - current.y) / current.scale;
    this.world.scale.set(scale);
    this.world.position.set(centerX - worldX * scale, centerY - worldY * scale);
    this.reportViewport();
  }

  destroy() {
    this.destroyed = true;
    for (const c of this.cleanup) c();
    this.cleanup = [];
    this.app?.destroy(true, { children: true });
    this.app = null;
    this.searchLayer?.remove();
    this.searchLayer = null;
    this.searchWorld = null;
    this.searchLabels.clear();
    this.replicaSearchLabels.clear();
  }

  /** Routes are recomputed only when the topology or the line style changes, not per frame. */
  private reroute() {
    const paths = routeEdges(this.edgeStyle, this.routeNodes, this.routeEdges, (node) => this.routeBoxForNode(node));
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
    if (this.searchWorld) this.searchWorld.style.transform = `translate(${this.world.x}px, ${this.world.y}px) scale(${s})`;
    this.host?.setAttribute("data-camera",JSON.stringify({x:this.world.x,y:this.world.y,scale:s}));
    const cardSize = this.serviceMetricsEnabled ? MAX_CARD_SIZE : COMPACT_CARD_SIZE;
    const r = cullRect({ x: -this.world.x / s, y: -this.world.y / s, width: app.screen.width / s, height: app.screen.height / s }, cardSize.width / 2 + 8);
    const displayScale = mapDisplayScale(this.serviceMetricsEnabled);
    const [x, y] = sourceMapPoint(r.x, r.y, displayScale);
    this.callbacks.onViewport(x, y, r.width / displayScale.x, r.height / displayScale.y, s);
    this.publishEdgeAnchors();
    this.publishReplicaAnchors();
  }

  /** Positions of replica cards in canvas pixels (`service@host:x,y`) for UI tests/debugging. */
  private publishReplicaAnchors() {
    const s = this.world.scale.x;
    const anchors = (this.infraView?.chips ?? []).map((c) =>
      `${encodeURIComponent(c.service_id)}@${encodeURIComponent(c.host_id)}:${c.x * s + this.world.x},${c.y * s + this.world.y}`,
    ).join(";");
    this.host?.setAttribute("data-replica-anchors", anchors);
    const frames = (this.infraView?.frames ?? []).map((frame) =>
      `${encodeURIComponent(frame.host_id)}:${(frame.x + frame.width / 2) * s + this.world.x},${(frame.y + FRAME_HEADER / 2) * s + this.world.y}`,
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
      if (m) parts.push(`${index}:${m[0] * s + this.world.x},${m[1] * s + this.world.y}`);
    }
    const next = parts.join(";");
    if (next === this.edgeAnchors) return;
    this.edgeAnchors = next;
    this.host?.setAttribute("data-edge-anchors", next);
  }

  /** Clicks anywhere on a card select it; the engine picks by the node centre. */
  private pickAt(wx: number, wy: number) {
    const f = this.frame;
    const displayScale = mapDisplayScale(this.serviceMetricsEnabled);
    if (f) {
      for (let i = 0; i < f.nodeIndices.length; i++) {
        const [x, y] = displayMapPoint(f.nodeData[i * NODE_STRIDE] ?? 0, f.nodeData[i * NODE_STRIDE + 1] ?? 0, displayScale);
        const size = this.cardSizeForNode(f.nodeIndices[i] ?? 0);
        if (Math.abs(wx - x) <= size.width / 2 && Math.abs(wy - y) <= size.height / 2) {
          const [sourceX, sourceY] = sourceMapPoint(x, y, displayScale);
          this.callbacks.onPick(sourceX, sourceY);
          return;
        }
      }
    }
    const chip = this.infraView?.chips.find((candidate) => {
      const size = this.cardSizeForNode(candidate.node);
      return Math.abs(candidate.x - wx) <= size.width / 2 && Math.abs(candidate.y - wy) <= size.height / 2;
    });
    if (chip) {
      if (this.callbacks.onPickInstance) this.callbacks.onPickInstance(chip.instance_id, chip.host_id, chip.service_id);
      else {
        const [sourceX, sourceY] = sourceMapPoint(chip.x, chip.y, displayScale);
        this.callbacks.onPick(sourceX, sourceY);
      }
      return;
    }
    const header = frameHeaderAt(this.infraView, wx, wy);
    if (header && this.callbacks.onPickHost) {
      this.callbacks.onPickHost(header.host_id);
      return;
    }
    if (this.callbacks.onPickEdge) {
      const paths = Array.from(this.edgeGeom, ([index, geometry]) => geometry.paths.map((path) => [index, path] as const)).flat();
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
    const [sourceX, sourceY] = sourceMapPoint(wx, wy, displayScale);
    this.callbacks.onPick(sourceX, sourceY);
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
    this.publishReplicaAnchors();
    for (const l of view.links) {
      for (const dash of zoomInvariantDashes(build(linkArc(l.from, l.to)))) line(front, dash);
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
    for (const text of this.metricTextPool.values()) {
      text.cpuLabel.visible = false;
      text.cpuValue.visible = false;
      text.memoryLabel.visible = false;
      text.memoryValue.visible = false;
      text.receive.visible = false;
      text.transmit.visible = false;
      text.incoming.visible = false;
      text.outgoing.visible = false;
    }
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
      const size = this.cardSizeForNode(c.node);
      const left = c.x - size.width / 2;
      const top = c.y - size.height / 2;
      const g = this.replicaCards;
      if (status > 0) g.roundRect(left - 3, top - 3, size.width + 6, size.height + 6, 15).stroke({ color, width: 6, alpha: (status >= 2 ? 0.22 : 0.1) * fade });
      g.roundRect(left, top, size.width, size.height, 12).fill({ color: CARD_BG, alpha: 0.96 * fade }).stroke({ color, width: 1.5, alpha: 0.85 * fade });
      if ((flags & FLAG_HIGHLIGHT) !== 0) g.roundRect(left - 5, top - 5, size.width + 10, size.height + 10, 16).stroke({ color: ACCENT, width: 2 });
      if (c.node === this.selectedIndex) g.roundRect(left - 8, top - 8, size.width + 16, size.height + 16, 18).stroke({ color: 0xffffff, width: 2, alpha: 0.9 });
      const icon = SERVICE_CARD_LAYOUT.icon;
      const expanded = this.serviceMetricsEnabled && this.kinds[c.node] === "service";
      const iconX = left + (expanded ? icon.x + icon.width / 2 : 27);
      const iconY = expanded ? top + icon.y + icon.height / 2 : c.y;
      g.roundRect(iconX - icon.width / 2, iconY - icon.height / 2, icon.width, icon.height, 9).fill({ color, alpha: 0.14 });
      drawKindIcon(g, this.kinds[c.node], (flags & FLAG_RESOURCE) !== 0, iconX, iconY, color);
      g.circle(left + size.width - 12, top + 12, 3.5).fill({ color: statusColor, alpha: status > 0 ? 1 : 0.6 });
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
      pooled.name.alpha = 0;
      pooled.sub.alpha = fade;
      const metrics = this.kinds[c.node] === "service" ? this.metricsForService(c.service_id, f) : null;
      if (metrics) {
        pooled.sub.visible = false;
        this.drawServiceMetricCard(g, key, c.service_id, metrics, left, top, fade, this.serviceMetricsLoading);
      } else {
        pooled.sub.visible = !this.serviceMetricsEnabled;
        if (this.serviceMetricsEnabled && serviceMetricPresentation(null, this.serviceMetricsLoading).loading) {
          this.drawServiceMetricSkeleton(g, left, top, fade);
        }
      }
      let searchLabel = this.replicaSearchLabels.get(key);
      if (!searchLabel) {
        searchLabel = this.createSearchLabel();
        this.replicaSearchLabels.set(key, searchLabel);
      }
      const searchName = this.names[c.node] ?? c.label;
      if (searchLabel.textContent !== searchName) searchLabel.textContent = searchName;
      if (metrics) {
        const description = this.metricDescription(metrics);
        searchLabel.title = description;
        searchLabel.setAttribute("aria-label", `Service replica ${searchName}. ${description}`);
      } else {
        searchLabel.title = "";
        searchLabel.setAttribute("aria-label", this.serviceMetricsLoading && this.serviceMetricsEnabled
          ? `Service replica ${searchName}. Loading service metrics`
          : `Service replica ${searchName}`);
      }
      searchLabel.style.left = `${left + SERVICE_CARD_LAYOUT.title.x}px`;
      searchLabel.style.top = `${top + SERVICE_CARD_LAYOUT.title.y}px`;
      searchLabel.style.opacity = String(fade);
      searchLabel.style.visibility = "visible";
      if (c.instance_ids.length > 1) {
        pooled.count.text = `×${c.instance_ids.length}`;
        pooled.count.visible = true;
        const width = pooled.count.width + 12;
        const x = left + size.width - width - 6;
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
        const searchLabel = this.replicaSearchLabels.get(key);
        if (searchLabel) searchLabel.style.visibility = "hidden";
      } else {
        text.name.visible = true;
        text.sub.visible = !this.serviceMetricsEnabled;
      }
    }
  }

  private selection(): MapSelection {
    return { node: this.selectedIndex, edge: this.selectedEdge, edgeNodes: this.selectedEdgeNodes, host: this.selectedHost };
  }

  private refreshSelectedIncidentEdges() {
    this.selectedIncidentEdges.clear();
    for (const index of incidentEdgeIndices(this.selectedIndex, this.routeEdges)) this.selectedIncidentEdges.add(index);
    this.host?.setAttribute("data-selected-incident-edges", [...this.selectedIncidentEdges].sort((a, b) => a - b).join(","));
  }

  private syncSelectionFade() {
    this.hops.alpha = selectionAlpha(this.selection());
  }

  private syncSearchNodes(nodes: TopologyView["nodes"]) {
    const present = new Set<number>();
    for (const node of nodes) {
      present.add(node.index);
      let label = this.searchLabels.get(node.index);
      if (!label) {
        label = this.createSearchLabel();
        this.searchLabels.set(node.index, label);
      }
      if (label.textContent !== node.name) label.textContent = node.name;
      label.dataset.nodeId = node.id;
      const size = this.cardSizeForNode(node.index);
      label.style.left = `${node.x - size.width / 2 + SERVICE_CARD_LAYOUT.title.x}px`;
      label.style.top = `${node.y - size.height / 2 + SERVICE_CARD_LAYOUT.title.y}px`;
    }
    for (const [index, label] of this.searchLabels) {
      if (!present.has(index)) {
        label.remove();
        this.searchLabels.delete(index);
      }
    }
  }

  private createSearchLabel(): HTMLDivElement {
    const label = document.createElement("div");
    label.setAttribute("role", "img");
    label.setAttribute("aria-label", "Service card");
    Object.assign(label.style, {
      position: "absolute",
      width: `${SERVICE_CARD_LAYOUT.title.width}px`,
      overflow: "hidden",
      color: "#e6ecf5",
      font: "600 13px/16px Inter, 'Segoe UI', system-ui, sans-serif",
      textOverflow: "ellipsis",
      whiteSpace: "nowrap",
      pointerEvents: "none",
      userSelect: "none",
    });
    this.searchWorld?.appendChild(label);
    return label;
  }

  private metricText(key: string): MetricCardText {
    const existing = this.metricTextPool.get(key);
    if (existing) return existing;
    const make = () => new Text({ text: "", style: this.metricStyle });
    const text = {
      cpuLabel: make(), cpuValue: make(), memoryLabel: make(), memoryValue: make(),
      receive: make(), transmit: make(), incoming: make(), outgoing: make(),
    };
    this.metricTextPool.set(key, text);
    this.labels.addChild(text.cpuLabel, text.cpuValue, text.memoryLabel, text.memoryValue, text.receive, text.transmit, text.incoming, text.outgoing);
    return text;
  }

  private metricDescription(metrics: ServiceCardMetrics): string {
    const percent = (ratio: number | null) => ratio === null ? "unavailable" : `${(ratio * 100).toFixed(1)}%`;
    const coverage = (metric: ServiceCardMetrics["cpu"]) => `${metric.measured}/${metric.total} replicas measured`;
    const parts: string[] = [];
    if (metrics.cpu.usage !== null) {
      parts.push(metrics.cpu.capacity === null
        ? `CPU ${metrics.cpu.usage.toFixed(2)} host logical cores`
        : `CPU ${metrics.cpu.usage.toFixed(2)}/${metrics.cpu.capacity.toFixed(2)} host logical cores (${percent(metrics.cpu.ratio)}; ${coverage(metrics.cpu)})`);
    }
    if (metrics.memory.usage !== null) {
      parts.push(metrics.memory.capacity === null
        ? `RAM ${formatBytes(metrics.memory.usage)}`
        : `RAM ${formatBytes(metrics.memory.usage)}/${formatBytes(metrics.memory.capacity)} (${metrics.memory.denominator ?? "capacity unknown"}; ${percent(metrics.memory.ratio)})`);
    }
    if (metrics.network.receiveBytesPerSecond !== null) parts.push(`RX ${formatBytesPerSecond(metrics.network.receiveBytesPerSecond)}`);
    if (metrics.network.transmitBytesPerSecond !== null) parts.push(`TX ${formatBytesPerSecond(metrics.network.transmitBytesPerSecond)}`);
    if (metrics.requests.incomingPerMinute !== null) parts.push(`Requests IN ${this.formatPerMinute(metrics.requests.incomingPerMinute)} per minute`);
    if (metrics.requests.outgoingPerMinute !== null) parts.push(`Requests OUT ${this.formatPerMinute(metrics.requests.outgoingPerMinute)} per minute`);
    return parts.join("; ") || "No service metrics collected";
  }

  private formatPerMinute(value: number | null): string {
    if (value === null || !Number.isFinite(value) || value < 0) return "—";
    if (value >= 10_000) return `${(value / 1000).toFixed(0)}k`;
    if (value >= 1000) return `${(value / 1000).toFixed(1)}k`;
    return `${Math.round(value)}`;
  }

  private drawServiceMetricCard(
    graphics: Graphics,
    key: string,
    _serviceId: string,
    metrics: ServiceCardMetrics,
    left: number,
    top: number,
    fade: number,
    loading: boolean,
  ) {
    const text = this.metricText(key);
    const presentation = serviceMetricPresentation(metrics, loading);
    const visible = new Set(presentation.rows);
    text.cpuLabel.visible = visible.has("cpu");
    text.cpuValue.visible = visible.has("cpu");
    text.memoryLabel.visible = visible.has("memory");
    text.memoryValue.visible = visible.has("memory");
    text.receive.visible = visible.has("receive");
    text.transmit.visible = visible.has("transmit");
    text.incoming.visible = visible.has("incoming");
    text.outgoing.visible = visible.has("outgoing");
    const percent = (ratio: number | null) => ratio === null ? "—" : `${Math.round(ratio * 100)}%`;
    const cpuPartial = metrics.cpu.measured < metrics.cpu.total ? ` ${metrics.cpu.measured}/${metrics.cpu.total}` : "";
    const memoryPartial = metrics.memory.measured < metrics.memory.total ? ` ${metrics.memory.measured}/${metrics.memory.total}` : "";
    text.cpuLabel.text = "CPU";
    text.cpuValue.text = `${metrics.cpu.ratio === null ? `${metrics.cpu.usage ?? 0} cores` : percent(metrics.cpu.ratio)}${cpuPartial}`;
    text.memoryLabel.text = "RAM";
    text.memoryValue.text = `${metrics.memory.ratio === null ? formatBytes(metrics.memory.usage) : percent(metrics.memory.ratio)}${memoryPartial}`;
    text.receive.text = `RX ${formatBytesPerSecond(metrics.network.receiveBytesPerSecond)}`;
    text.transmit.text = `TX ${formatBytesPerSecond(metrics.network.transmitBytesPerSecond)}`;
    text.incoming.text = `IN ${this.formatPerMinute(metrics.requests.incomingPerMinute)}/m`;
    text.outgoing.text = `OUT ${this.formatPerMinute(metrics.requests.outgoingPerMinute)}/m`;
    const layout = SERVICE_CARD_LAYOUT;
    text.cpuLabel.position.set(left + layout.cpu.label.x, top + layout.cpu.label.y);
    text.cpuValue.position.set(left + layout.cpu.value.x, top + layout.cpu.value.y);
    text.memoryLabel.position.set(left + layout.memory.label.x, top + layout.memory.label.y);
    text.memoryValue.position.set(left + layout.memory.value.x, top + layout.memory.value.y);
    text.receive.position.set(left + layout.network.receive.x, top + layout.network.receive.y);
    text.transmit.position.set(left + layout.network.transmit.x, top + layout.network.transmit.y);
    text.incoming.position.set(left + layout.requests.incoming.x, top + layout.requests.incoming.y);
    text.outgoing.position.set(left + layout.requests.outgoing.x, top + layout.requests.outgoing.y);
    const drawBar = (y: number, ratio: number | null) => {
      const model = metricBar(ratio);
      const segmentGap = 2;
      const segmentWidth = 7;
      for (let segment = 0; segment < model.segments; segment += 1) {
        const color = segment < model.filled ? model.color ?? METRIC_BAR_COLORS.empty : METRIC_BAR_COLORS.empty;
        graphics.roundRect(left + 43 + segment * (segmentWidth + segmentGap), y, segmentWidth, 6, 2)
          .fill({ color, alpha: fade });
      }
    };
    if (visible.has("cpu") && metrics.cpu.ratio !== null) drawBar(top + layout.cpu.barY, metrics.cpu.ratio);
    if (visible.has("memory") && metrics.memory.ratio !== null) drawBar(top + layout.memory.barY, metrics.memory.ratio);
    if (presentation.loadingRows.length) this.drawServiceMetricSkeleton(graphics, left, top, fade, presentation.loadingRows);
  }

  private drawServiceMetricSkeleton(graphics: Graphics, left: number, top: number, fade: number, rows = ["cpu", "memory", "receive", "transmit", "incoming", "outgoing"]) {
    const pulse = 0.35 + (Math.sin(performance.now() / 420) + 1) * 0.12;
    const line = (x: number, y: number, width: number, height = 5) => {
      graphics.roundRect(left + x, top + y, width, height, height / 2).fill({ color: 0x8492a6, alpha: pulse * fade });
    };
    const layout = SERVICE_CARD_LAYOUT;
    if (rows.includes("cpu")) { line(layout.cpu.label.x, layout.cpu.label.y, 23); line(43, layout.cpu.barY, 84); line(layout.cpu.value.x, layout.cpu.value.y, 40); }
    if (rows.includes("memory")) { line(layout.memory.label.x, layout.memory.label.y, 23); line(43, layout.memory.barY, 84); line(layout.memory.value.x, layout.memory.value.y, 40); }
    if (rows.includes("receive") || rows.includes("transmit")) {
      if (rows.includes("receive")) line(layout.network.receive.x, layout.network.receive.y, 70);
      if (rows.includes("transmit")) line(layout.network.transmit.x, layout.network.transmit.y, 78);
    }
    if (rows.includes("incoming")) line(layout.requests.incoming.x, layout.requests.incoming.y, 70);
    if (rows.includes("outgoing")) line(layout.requests.outgoing.x, layout.requests.outgoing.y, 78);
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
    const displayScale = mapDisplayScale(this.serviceMetricsEnabled);
    const replicaPoints = (this.infraView?.chips ?? []).map((chip) => ({ node: chip.node, x: chip.x, y: chip.y }));
    const cardObstacles = [...this.routeNodes.map(({ index, x, y }) => ({ node: index, x, y })), ...replicaPoints]
      .map((card) => ({ ...card, box: this.routeBoxForNode(card.node) }));
    let replicaPathCount = 0;
    let replicaBranchCount = 0;
    for (let i = 0; i < f.edgeIndices.length; i++) {
      const o = i * EDGE_STRIDE;
      const [sx, sy] = displayMapPoint(f.edgeData[o] ?? 0, f.edgeData[o + 1] ?? 0, displayScale);
      const [tx, ty] = displayMapPoint(f.edgeData[o + 2] ?? 0, f.edgeData[o + 3] ?? 0, displayScale);
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
      const incident = this.selectedIncidentEdges.has(index);
      const selected = this.selectedEdge === index;
      const focused = selected || incident;
      const fade = edgeFadeAlpha(focused, selection);
      const route = this.routes.get(index);
      // A route is only valid for the positions it was computed for (topology may lag a frame).
      const fresh = route && near(route.sx, sx) && near(route.sy, sy) && near(route.tx, tx) && near(route.ty, ty);
      const routeSpec = this.routeEdges[index] && this.routeEdges[index]?.index === index ? this.routeEdges[index] : this.routeEdges.find((candidate) => candidate.index === index);
      const sourceBox = routeSpec ? this.routeBoxForNode(routeSpec.source) : routeBoxForCard(MAX_CARD_SIZE);
      const targetBox = routeSpec ? this.routeBoxForNode(routeSpec.target) : sourceBox;
      const groups = routeSpec
        ? edgeCardGroups({ node: routeSpec.source, x: sx, y: sy }, { node: routeSpec.target, x: tx, y: ty }, replicaPoints)
        : [];
      const routedPaths: Array<{ path: EdgePath; isPrimary: boolean }> = [];
      const trunks: EdgePath[] = [];
      for (const group of groups) {
        const branch = edgeBranchPaths(group, this.edgeStyle, (node) => this.routeBoxForNode(node), cardObstacles);
        if (branch) {
          replicaBranchCount++;
          trunks.push(branch.trunk);
          routedPaths.push(...branch.branches);
          continue;
        }
        // A multi-replica relationship must not fall back to a direct route: that path can
        // cross an intervening card when the obstacle-aware visibility grid has no route.
        if (group.targets.length > 1) continue;
        for (const target of group.targets) {
          const path = group.isPrimary && target.isPrimary
            ? fresh ? route.path : routeBetweenCards(group.source, sourceBox, target.point, targetBox, cardObstacles, this.edgeStyle)
            : routeBetweenCards(
                group.source,
                this.routeBoxForNode(group.source.node),
                target.point,
                this.routeBoxForNode(target.point.node),
                cardObstacles,
                this.edgeStyle,
              );
          if (path) routedPaths.push({ path, isPrimary: group.isPrimary && target.isPrimary });
        }
      }
      if (routedPaths.length === 0 && trunks.length === 0) {
        const fallback = fresh ? route.path : routeBetweenCards(
          { node: routeSpec?.source ?? -1, x: sx, y: sy },
          sourceBox,
          { node: routeSpec?.target ?? -1, x: tx, y: ty },
          targetBox,
          cardObstacles,
          this.edgeStyle,
        );
        if (fallback) routedPaths.push({ path: fallback, isPrimary: true });
      }
      const paths = [...trunks, ...routedPaths.map(({ path }) => path)];
      if (paths.length === 0) continue;
      if (paths.length > 1) replicaPathCount += paths.length - 1;
      const primaryPath = routedPaths.find((entry) => entry.isPrimary)?.path ?? routedPaths[0]?.path ?? paths[0];
      if (!primaryPath) continue;
      this.edgeGeom.set(index, { path: primaryPath, paths, trunks, color });
      const width = highlight ? 2.5 : 1.25 + Math.min(1.5, Math.log10(1 + calls) * 0.6);
      const e = focused || (flags & FLAG_DIMMED) === 0 ? this.edges : this.edgesDim;
      const drawPath = (path: EdgePath, hasArrow: boolean) => {
        if (index === this.selectedEdge) line(this.edges, path.pts).stroke({ color: EDGE_SELECTED, width: 10, alpha: 0.35, ...LINE_STYLE });
        if (incident && !selected) {
          line(this.edges, path.pts).stroke({
            color: EDGE_SELECTED,
            width: incidentGlowWidth(width, inferred),
            alpha: inferred ? 0.34 : 0.32,
            ...LINE_STYLE,
          });
        }
        if (inferred) {
          for (const dash of zoomInvariantDashes(path)) line(e, dash);
          e.stroke({ color, width, alpha: 0.95 * fade, ...LINE_STYLE });
        } else {
          line(e, path.pts).stroke({ color, width: width + 6, alpha: (calls > 0 ? 0.14 : 0.08) * fade, ...LINE_STYLE });
          line(e, path.pts).stroke({ color, width, alpha: (calls > 0 ? 1 : 0.72) * fade, ...LINE_STYLE });
        }
        if (hasArrow) arrow(e, path, incident ? EDGE_SELECTED : color, fade);
      };
      for (const trunk of trunks) drawPath(trunk, false);
      for (const entry of routedPaths) drawPath(entry.path, true);
    }
    this.host?.setAttribute("data-replica-edge-paths", String(replicaPathCount));
    this.host?.setAttribute("data-replica-edge-branches", String(replicaBranchCount));
    this.publishEdgeAnchors();
  }

  private drawNodes(f: Frame) {
    this.nodes.clear();
    this.nodesDim.clear();
    for (const text of this.countTextPool.values()) text.visible = false;
    for (const label of this.searchLabels.values()) label.style.visibility = "hidden";
    const seen = new Set<number>();
    const metricDescriptions: Record<string, string> = {};
    const selection = this.selection();
    const displayScale = mapDisplayScale(this.serviceMetricsEnabled);
    for (let i = 0; i < f.nodeIndices.length; i++) {
      const o = i * NODE_STRIDE;
      const [x, y] = displayMapPoint(f.nodeData[o] ?? 0, f.nodeData[o + 1] ?? 0, displayScale);
      const status = f.nodeData[o + 3] ?? 0;
      const active = f.nodeData[o + 4] ?? 0;
      const requests = f.nodeData[o + 5] ?? 0;
      const flags = f.nodeData[o + 6] ?? 0;
      const idx = f.nodeIndices[i] ?? 0;
      const size = this.cardSizeForNode(idx);
      const hw = size.width / 2;
      const hh = size.height / 2;
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
        n.roundRect(left - 3, top - 3, size.width + 6, size.height + 6, 15).stroke({ color, width: 6, alpha: (status >= 2 ? 0.22 : 0.1) * fade });
      }
      if (external) {
        // Outside the project: dashed border, no solid frame, so it never reads as one of our services.
        n.roundRect(left, top, size.width, size.height, 4).fill({ color: CARD_BG, alpha: 0.9 * fade });
        for (const dash of dashedRect(left, top, size.width, size.height)) line(n, dash);
        n.stroke({ color, width: 1.75, alpha: 0.9 * fade, ...LINE_STYLE });
      } else {
        n.roundRect(left, top, size.width, size.height, 12)
          .fill({ color: CARD_BG, alpha: 0.96 * fade })
          .stroke({ color, width: 1.5, alpha: (inferred ? 0.45 : 0.85) * fade });
      }
      if ((flags & FLAG_HIGHLIGHT) !== 0) n.roundRect(left - 5, top - 5, size.width + 10, size.height + 10, 16).stroke({ color: ACCENT, width: 2 });
      if (idx === this.selectedIndex) n.roundRect(left - 8, top - 8, size.width + 16, size.height + 16, 18).stroke({ color: 0xffffff, width: 2, alpha: 0.9 });

      const instanceCount = this.kinds[idx] === "service" ? this.instanceCounts.get(this.ids[idx] ?? "") ?? 0 : 0;
      if (instanceCount > 1) {
        let badge = this.countTextPool.get(idx);
        if (!badge) {
          badge = new Text({ text: "", style: this.chipStyle });
          this.countTextPool.set(idx, badge);
          this.labels.addChild(badge);
        }
        const badgeValue = `×${instanceCount}`;
        if (badge.text !== badgeValue) badge.text = badgeValue;
        badge.visible = true;
        const badgeWidth = badge.width + 14;
        const badgeX = left + size.width - badgeWidth + 6;
        const badgeY = top - 11;
        const badgeFade = nodeFocused(idx, this.hostForNode(idx), selection) ? 1 : SELECTION_DIM_ALPHA;
        n.roundRect(badgeX, badgeY, badgeWidth, 20, 10).fill({ color: 0x4c1d95, alpha: 0.95 * badgeFade }).stroke({ color: 0xa78bfa, width: 1, alpha: badgeFade });
        badge.alpha = badgeFade;
        badge.position.set(badgeX + 7, badgeY + 3);
      }

      const metricCard = this.serviceMetricsEnabled && this.kinds[idx] === "service";
      const icon = SERVICE_CARD_LAYOUT.icon;
      const ix = left + (metricCard ? icon.x : 12);
      const iy = metricCard ? top + icon.y : y - 15;
      n.roundRect(ix, iy, icon.width, icon.height, 9).fill({ color, alpha: 0.14 });
      const resource = (flags & FLAG_RESOURCE) !== 0;
      drawKindIcon(n, this.kinds[idx], resource, ix + icon.width / 2, iy + icon.height / 2, color);
      n.circle(left + size.width - 12, top + 12, 3.5).fill({ color: statusColor, alpha: status > 0 ? 1 : 0.6 });

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
      const metrics = this.kinds[idx] === "service" ? this.metricsForService(this.ids[idx] ?? "", f) : null;
      t.sub.visible = !(this.serviceMetricsEnabled && this.kinds[idx] === "service");
      t.name.alpha = 0;
      t.sub.alpha = (dimmed ? DIM_ALPHA + 0.1 : 1) * fade;
      t.name.position.set(left + SERVICE_CARD_LAYOUT.title.x, top + SERVICE_CARD_LAYOUT.title.y);
      t.sub.position.set(left + 52, y + 2);
      if (metrics && this.serviceMetricsEnabled) {
        this.drawServiceMetricCard(n, `service:${idx}`, this.ids[idx] ?? "", metrics, left, top, (dimmed ? DIM_ALPHA + 0.1 : 1) * fade, this.serviceMetricsLoading);
        const description = this.metricDescription(metrics);
        metricDescriptions[this.ids[idx] ?? String(idx)] = description;
        const searchLabel = this.searchLabels.get(idx);
        if (searchLabel) {
          searchLabel.title = description;
          searchLabel.setAttribute("aria-label", `Service ${this.names[idx] ?? ""}. ${description}${instanceCount > 1 ? `; ${instanceCount} containers` : ""}`);
        }
      } else {
        if (this.serviceMetricsEnabled && this.kinds[idx] === "service" && serviceMetricPresentation(null, this.serviceMetricsLoading).loading) {
          this.drawServiceMetricSkeleton(n, left, top, (dimmed ? DIM_ALPHA + 0.1 : 1) * fade);
        }
        const searchLabel = this.searchLabels.get(idx);
        if (searchLabel) {
          searchLabel.title = "";
          searchLabel.setAttribute("aria-label", this.serviceMetricsLoading && this.serviceMetricsEnabled && this.kinds[idx] === "service"
            ? `Service ${this.names[idx] ?? ""}. Loading service metrics`
            : instanceCount > 1
              ? `Service ${this.names[idx] ?? ""}. ${instanceCount} containers`
              : `Service ${this.names[idx] ?? ""}`);
        }
      }
      const searchLabel = this.searchLabels.get(idx);
      if (searchLabel) {
        searchLabel.style.opacity = String((dimmed ? DIM_ALPHA + 0.1 : 1) * fade);
        searchLabel.style.visibility = "visible";
      }
    }
    for (const [idx, t] of this.textPool) {
      if (!seen.has(idx)) {
        t.name.visible = false;
        t.sub.visible = false;
      }
    }
    // Which cards carry a visible label, for tests (node indices).
    this.host?.setAttribute("data-labelled-nodes", Array.from(seen).join(","));
    this.host?.setAttribute("data-service-metrics", JSON.stringify(metricDescriptions));
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
