/**
 * Canvas tools.
 *
 * Each tool is a small state machine over pointer events, kept out of React so
 * a drag does not re-render the chrome sixty times a second. A tool never
 * mutates the document: it produces a `Command`, and the caller runs it through
 * the history stack.
 */

import type { Command } from '../doc/commands';
import { AddWall, newId } from '../doc/commands';

export type ToolId = 'select' | 'wall' | 'zone' | 'door';

export interface Point {
  x: number;
  y: number;
}

/** Snapping configuration. Grid is metres; the rest are screen-independent. */
export interface SnapConfig {
  /** Grid spacing in metres. 0 disables grid snapping. */
  gridM: number;
  /** Snap to an existing vertex within this many metres. */
  vertexM: number;
  /** Constrain to 0/45/90 degrees while held. */
  ortho: boolean;
}

export const DEFAULT_SNAP: SnapConfig = {
  gridM: 0.5,
  vertexM: 0.35,
  ortho: false,
};

/**
 * Resolve a raw world position to a snapped one.
 *
 * Vertex snapping wins over grid snapping: a user aiming at an existing corner
 * means that corner, and landing 3 mm away would leave a gap that the compiler
 * later reports as an unclosed outline. Closing wall runs exactly is the whole
 * point of snapping in this product.
 */
export function snap(p: Point, vertices: Point[], cfg: SnapConfig, anchor?: Point): Point {
  let out = { ...p };

  // Ortho constraint applies before snapping, so the result stays on the axis.
  if (cfg.ortho && anchor) {
    const dx = out.x - anchor.x;
    const dy = out.y - anchor.y;
    const angle = Math.atan2(dy, dx);
    const step = Math.PI / 4;
    const locked = Math.round(angle / step) * step;
    const len = Math.hypot(dx, dy);
    out = { x: anchor.x + Math.cos(locked) * len, y: anchor.y + Math.sin(locked) * len };
  }

  let best: { d: number; p: Point } | null = null;
  for (const v of vertices) {
    const d = Math.hypot(v.x - out.x, v.y - out.y);
    if (d <= cfg.vertexM && (!best || d < best.d)) best = { d, p: v };
  }
  if (best) return { ...best.p };

  if (cfg.gridM > 0) {
    return {
      x: Math.round(out.x / cfg.gridM) * cfg.gridM,
      y: Math.round(out.y / cfg.gridM) * cfg.gridM,
    };
  }
  return out;
}

/**
 * Wall drawing: click to place each vertex, double-click or Enter to finish,
 * Escape to abandon.
 *
 * A wall is a polyline rather than a single segment so that a run of connected
 * wall stays one editable entity — which is also how imported geometry arrives.
 */
export class WallTool {
  private points: Point[] = [];
  /** Where the pointer currently is, for the rubber-band preview. */
  private cursor: Point | null = null;

  get isDrawing(): boolean {
    return this.points.length > 0;
  }

  /** Committed points plus the live cursor, for previewing. */
  preview(): Point[] {
    if (!this.points.length) return [];
    return this.cursor ? [...this.points, this.cursor] : [...this.points];
  }

  moveTo(p: Point): void {
    this.cursor = p;
  }

  addPoint(p: Point): void {
    const last = this.points.at(-1);
    // Ignore a repeat click on the same spot: it would create a zero-length
    // segment, which the compiler reports as a degenerate wall.
    if (last && Math.hypot(last.x - p.x, last.y - p.y) < 1e-6) return;
    this.points.push({ ...p });
  }

  cancel(): void {
    this.points = [];
    this.cursor = null;
  }

  /**
   * Finish the run and produce the command, or `null` if there is nothing to
   * commit. Fewer than two points is not a wall.
   */
  finish(floorId: string, thicknessM = 0.2): Command | null {
    const pts = this.points;
    this.points = [];
    this.cursor = null;
    if (pts.length < 2) return null;

    return new AddWall(floorId, {
      id: newId('w'),
      polyline: pts.map((p): [number, number] => [p.x, p.y]),
      thicknessM,
      kind: 'structural',
      permeable: false,
    });
  }
}

/** Every vertex in the document, for vertex snapping. */
export function collectVertices(doc: {
  floors: Array<{
    id: string;
    walls?: Array<{ polyline: number[][] }>;
    zones?: Array<{ polygon: number[][] }>;
  }>;
}, floorId: string): Point[] {
  const floor = doc.floors.find((f) => f.id === floorId);
  if (!floor) return [];
  const out: Point[] = [];
  for (const w of floor.walls ?? []) {
    for (const p of w.polyline) out.push({ x: p[0]!, y: p[1]! });
  }
  for (const z of floor.zones ?? []) {
    for (const p of z.polygon) out.push({ x: p[0]!, y: p[1]! });
  }
  return out;
}
