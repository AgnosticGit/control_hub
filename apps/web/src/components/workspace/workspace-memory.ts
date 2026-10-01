import type { MapCamera } from "@/renderer/pixi-map";
import type { View } from "./timeline/scale";

/** View state of one workspace that outlives its map and timeline while another page is shown. */
export class WorkspaceMemory {
  private mapCamera: MapCamera | null = null;
  private zoom: View | null = null;

  camera(): MapCamera | null {
    return this.mapCamera;
  }

  saveCamera(camera: MapCamera | null) {
    this.mapCamera = camera && [camera.x, camera.y, camera.scale].every(Number.isFinite) && camera.scale > 0 ? camera : null;
  }

  /** The user's timeline zoom; null means "fit the whole range". */
  timelineView(): View | null {
    return this.zoom;
  }

  saveTimelineView(view: View | null) {
    this.zoom = view;
  }
}
