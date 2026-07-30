/**
 * Canvas tools.
 *
 * Each tool is a small state machine over pointer events, kept out of React so
 * a drag does not re-render the chrome sixty times a second. A tool never
 * mutates the document: it produces a `Command`, and the caller runs it through
 * the history stack.
 */

import type { Command } from '../doc/commands';
import { AddOpening, AddWall, AddZone, newId } from '../doc/commands';
import type { VenueDoc, ZoneKind } from '../schema/venue';

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


// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

function dist(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

/** Closest point on segment `a—b` to `p`, and how far along it that is. */
function closestOnSegment(a: Point, b: Point, p: Point): { point: Point; t: number; d: number } {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len2 = dx * dx + dy * dy;
  const t = len2 <= 1e-12 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2));
  const point = { x: a.x + dx * t, y: a.y + dy * t };
  return { point, t, d: dist(point, p) };
}

/**
 * Project a point onto a wall polyline, returning the normalised arc-length
 * position along it.
 *
 * `t` is normalised *arc length*, not segment index — an opening is stored that
 * way so it survives its wall being reshaped, and the engine resolves it the
 * same way (`Polyline::point_at`). Computing it any other way here would put
 * doors in the wrong place on multi-segment walls.
 */
export function projectOntoWall(
  polyline: number[][],
  p: Point,
): { t: number; point: Point; distance: number } | null {
  if (polyline.length < 2) return null;

  const pts: Point[] = polyline.map((q) => ({ x: q[0]!, y: q[1]! }));
  const lengths: number[] = [];
  let total = 0;
  for (let i = 0; i + 1 < pts.length; i++) {
    const seg = dist(pts[i]!, pts[i + 1]!);
    lengths.push(seg);
    total += seg;
  }
  if (total <= 1e-12) return null;

  let best: { t: number; point: Point; distance: number } | null = null;
  let walked = 0;
  for (let i = 0; i + 1 < pts.length; i++) {
    const r = closestOnSegment(pts[i]!, pts[i + 1]!, p);
    if (!best || r.d < best.distance) {
      best = { t: (walked + r.t * lengths[i]!) / total, point: r.point, distance: r.d };
    }
    walked += lengths[i]!;
  }
  return best;
}

/** What the user clicked, if anything. */
export type Selection =
  | { kind: 'wall'; id: string }
  | { kind: 'zone'; id: string }
  | { kind: 'opening'; id: string };

/**
 * Hit-test a click against the document.
 *
 * Openings win over walls because an opening sits *on* a wall and is the
 * smaller target; walls win over zones because a zone covers a large area and
 * would otherwise swallow every click inside it.
 */
export function hitTest(
  doc: VenueDoc,
  floorId: string,
  p: Point,
  toleranceM: number,
): Selection | null {
  const floor = doc.floors.find((f) => f.id === floorId);
  if (!floor) return null;

  for (const o of floor.openings ?? []) {
    const wall = (floor.walls ?? []).find((w) => w.id === o.wall);
    if (!wall) continue;
    const proj = projectOntoWall(wall.polyline, p);
    if (!proj) continue;
    // Within the doorway's own span, not merely near its wall.
    const span = o.widthM / 2;
    const along = Math.abs(proj.t - o.t) * polylineLength(wall.polyline);
    if (proj.distance <= toleranceM && along <= span) {
      return { kind: 'opening', id: o.id };
    }
  }

  for (const w of floor.walls ?? []) {
    const proj = projectOntoWall(w.polyline, p);
    if (proj && proj.distance <= toleranceM) return { kind: 'wall', id: w.id };
  }

  for (const z of floor.zones ?? []) {
    if (pointInPolygon(z.polygon, p)) return { kind: 'zone', id: z.id };
  }
  return null;
}

export function polylineLength(polyline: number[][]): number {
  let total = 0;
  for (let i = 0; i + 1 < polyline.length; i++) {
    total += Math.hypot(
      polyline[i + 1]![0]! - polyline[i]![0]!,
      polyline[i + 1]![1]! - polyline[i]![1]!,
    );
  }
  return total;
}

function pointInPolygon(poly: number[][], p: Point): boolean {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const xi = poly[i]![0]!;
    const yi = poly[i]![1]!;
    const xj = poly[j]![0]!;
    const yj = poly[j]![1]!;
    if (yi > p.y !== yj > p.y && p.x < ((xj - xi) * (p.y - yi)) / (yj - yi) + xi) {
      inside = !inside;
    }
  }
  return inside;
}

// ---------------------------------------------------------------------------
// Zone tool
// ---------------------------------------------------------------------------

/** Polygon drawing. Same interaction as the wall tool, closed rather than open. */
export class ZoneTool {
  private points: Point[] = [];
  private cursor: Point | null = null;

  get isDrawing(): boolean {
    return this.points.length > 0;
  }

  preview(): Point[] {
    if (!this.points.length) return [];
    return this.cursor ? [...this.points, this.cursor] : [...this.points];
  }

  moveTo(p: Point): void {
    this.cursor = p;
  }

  addPoint(p: Point): void {
    const last = this.points.at(-1);
    if (last && dist(last, p) < 1e-6) return;
    this.points.push({ ...p });
  }

  cancel(): void {
    this.points = [];
    this.cursor = null;
  }

  /** A ring needs three points; fewer is a line, not an area. */
  finish(floorId: string, kind: ZoneKind = 'assemblyConcentrated'): Command | null {
    const pts = this.points;
    this.points = [];
    this.cursor = null;
    if (pts.length < 3) return null;

    return new AddZone(floorId, {
      id: newId('z'),
      polygon: pts.map((p): [number, number] => [p.x, p.y]),
      kind,
      speedMultiplier: 1,
      isVoid: false,
    });
  }
}

// ---------------------------------------------------------------------------
// Door tool
// ---------------------------------------------------------------------------

/**
 * Place a doorway by clicking on a wall.
 *
 * The click is projected onto the nearest wall and stored as a normalised
 * position along it, which is how the schema represents an opening — so moving
 * or reshaping that wall later carries the door with it.
 */
export class DoorTool {
  /** Nearest wall under the cursor, for previewing before commit. */
  hover(doc: VenueDoc, floorId: string, p: Point, toleranceM = 0.6):
    | { wallId: string; t: number; point: Point }
    | null {
    const floor = doc.floors.find((f) => f.id === floorId);
    if (!floor) return null;

    let best: { wallId: string; t: number; point: Point; d: number } | null = null;
    for (const w of floor.walls ?? []) {
      const proj = projectOntoWall(w.polyline, p);
      if (!proj || proj.distance > toleranceM) continue;
      if (!best || proj.distance < best.d) {
        best = { wallId: w.id, t: proj.t, point: proj.point, d: proj.distance };
      }
    }
    return best ? { wallId: best.wallId, t: best.t, point: best.point } : null;
  }

  place(floorId: string, wallId: string, t: number, widthM = 1.8, isFireExit = true): Command {
    return new AddOpening(floorId, {
      id: newId('op'),
      wall: wallId,
      t,
      widthM,
      kind: 'doubleDoor',
      swing: 'both',
      isFireExit,
      capacityFactor: 1,
    });
  }
}
