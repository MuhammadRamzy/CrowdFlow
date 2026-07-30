/**
 * The canvas scene.
 *
 * Deliberately outside React (docs/01-architecture.md §3). React owns the
 * chrome; PixiJS owns the scene. Driving 25,000 agents through a React render
 * would spend the entire frame budget on reconciliation before drawing a single
 * pixel.
 *
 * Agents are drawn as one instanced particle batch. Venue geometry is static
 * and rebuilt only when the venue changes, not per frame.
 */

import {
  Application,
  Container,
  Graphics,
  Particle,
  ParticleContainer,
  Sprite,
  Texture,
} from 'pixi.js';
import type { DensityField, VenueGeometry } from '../engine/bridge';
import { AgentState } from '../engine/bridge';

/** Reads the palette from CSS so tokens.css stays the single source of colour. */
function token(name: string, fallback: number): number {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  if (!raw.startsWith('#')) return fallback;
  return parseInt(raw.slice(1), 16);
}

export interface ViewportState {
  /** Pixels per metre. */
  scale: number;
  /** World-space centre of the view. */
  cx: number;
  cy: number;
}

export class Renderer {
  private app!: Application;
  private world = new Container();
  private gridLayer = new Graphics();
  private floorLayer = new Graphics();
  private wallLayer = new Graphics();
  private doorLayer = new Graphics();
  private heatLayer = new Container();
  private heatSprite: Sprite | null = null;
  private heatCanvas: HTMLCanvasElement | null = null;
  private ramp: Uint8Array | null = null;
  private previewLayer = new Graphics();
  private snapLayer = new Graphics();
  private selectLayer = new Graphics();
  /** Set by the host so tools receive world-space pointer events. */
  onWorldPointer: ((p: { x: number; y: number }, kind: 'move' | 'down' | 'dblclick') => void) | null =
    null;
  /** True while a drawing tool is active; suppresses pan-drag. */
  drawing = false;
  private agents!: ParticleContainer;
  private particles: Particle[] = [];
  private dot!: Texture;

  private view: ViewportState = { scale: 24, cx: 0, cy: 0 };
  private geometry: VenueGeometry | null = null;
  private colours = {
    grid: 0x1a2331,
    floor: 0x141d29,
    wall: 0x9fb2c8,
    door: 0x3dd68c,
    walking: 0x58c4e8,
    blocked: 0xffb020,
    select: 0x58c4e8,
  };

  async mount(host: HTMLElement): Promise<void> {
    this.colours = {
      grid: token('--line', 0x263140),
      floor: token('--sunken', 0x0f141b),
      wall: token('--chalk', 0xc9d3df),
      door: token('--normal', 0x3dd68c),
      walking: token('--select', 0x58c4e8),
      blocked: token('--supervise', 0xffb020),
      select: token('--select', 0x58c4e8),
    };

    this.app = new Application();
    await this.app.init({
      background: token('--void', 0x0b0e13),
      antialias: true,
      resizeTo: host,
      preference: 'webgl',
    });
    host.appendChild(this.app.canvas);

    // A 1-pixel white dot, tinted per agent. One texture, one draw call.
    const g = new Graphics().circle(0, 0, 8).fill(0xffffff);
    this.dot = this.app.renderer.generateTexture(g);

    this.agents = new ParticleContainer({
      dynamicProperties: { position: true, tint: true, scale: false, rotation: false },
    });

    this.buildRamp();
    // Heat sits above the floor but below walls and agents: a heatmap that
    // covered the walls would hide the geometry causing the congestion.
    this.world.addChild(
      this.gridLayer,
      this.floorLayer,
      this.heatLayer,
      this.wallLayer,
      this.doorLayer,
      this.agents,
      this.previewLayer,
      this.selectLayer,
      this.snapLayer,
    );
    this.app.stage.addChild(this.world);

    this.installInput(host);
    this.app.renderer.on('resize', () => this.redrawStatic());
  }

  destroy(): void {
    this.app?.destroy(true, { children: true });
  }

  /**
   * Build the density colour ramp once, as a 256-entry RGBA lookup.
   *
   * The stops are the crowd-science bands, not an aesthetic gradient: the
   * colour changes where the *meaning* changes, so a reader can tell 4 p/m²
   * from 6 p/m² without consulting a legend. Alpha rises with density too, so
   * empty floor stays legible underneath.
   */
  private buildRamp(): void {
    const stops: Array<{ at: number; c: [number, number, number] }> = [
      { at: 0.0, c: [27, 58, 92] },    // < 1   free flow
      { at: 1.0, c: [46, 125, 154] },  // 1-2   comfortable
      { at: 2.0, c: [70, 176, 138] },  // 2-3   steady
      { at: 4.0, c: [196, 192, 74] },  // 3-4   restricted
      { at: 6.0, c: [224, 138, 60] },  // 4-6   dense
      { at: 8.0, c: [212, 63, 63] },   // 6+    critical
    ];
    const scale = 8; // must match DENSITY_SCALE in cf-wasm
    const ramp = new Uint8Array(256 * 4);
    for (let i = 0; i < 256; i++) {
      const d = (i / 255) * scale;
      let lo = stops[0]!;
      let hi = stops[stops.length - 1]!;
      for (let k = 0; k < stops.length - 1; k++) {
        if (d >= stops[k]!.at && d <= stops[k + 1]!.at) {
          lo = stops[k]!;
          hi = stops[k + 1]!;
          break;
        }
      }
      const span = hi.at - lo.at || 1;
      const t = Math.min(1, Math.max(0, (d - lo.at) / span));
      ramp[i * 4 + 0] = Math.round(lo.c[0] + (hi.c[0] - lo.c[0]) * t);
      ramp[i * 4 + 1] = Math.round(lo.c[1] + (hi.c[1] - lo.c[1]) * t);
      ramp[i * 4 + 2] = Math.round(lo.c[2] + (hi.c[2] - lo.c[2]) * t);
      // Fade in from nothing so an empty venue shows bare floor, not a wash of
      // blue that reads as "occupied".
      ramp[i * 4 + 3] = Math.round(Math.min(1, (i / 255) * 3.2) * 210);
    }
    this.ramp = ramp;
  }

  /** Upload a density field, or hide the overlay when `field` is null. */
  setDensity(field: DensityField | null): void {
    if (!field || !this.ramp) {
      if (this.heatSprite) this.heatSprite.visible = false;
      return;
    }

    const { cols, rows, bytes } = field;
    if (!this.heatCanvas || this.heatCanvas.width !== cols || this.heatCanvas.height !== rows) {
      this.heatCanvas = document.createElement('canvas');
      this.heatCanvas.width = cols;
      this.heatCanvas.height = rows;
      this.heatSprite?.destroy();
      this.heatSprite = new Sprite(Texture.from(this.heatCanvas));
      this.heatLayer.removeChildren();
      this.heatLayer.addChild(this.heatSprite);
    }

    const ctx = this.heatCanvas.getContext('2d');
    if (!ctx) return;
    const img = ctx.createImageData(cols, rows);
    for (let i = 0; i < bytes.length; i++) {
      const v = bytes[i]! * 4;
      const o = i * 4;
      img.data[o] = this.ramp[v]!;
      img.data[o + 1] = this.ramp[v + 1]!;
      img.data[o + 2] = this.ramp[v + 2]!;
      img.data[o + 3] = this.ramp[v + 3]!;
    }
    ctx.putImageData(img, 0, 0);
    this.heatSprite!.texture.source.update();

    // The grid is row-major from its origin upward, but screen y runs down, so
    // the sprite is flipped rather than the data — flipping the data would cost
    // a copy every frame.
    const s = this.heatSprite!;
    s.visible = true;
    s.width = cols * field.cell;
    s.height = -rows * field.cell;
    s.position.set(field.originX, -field.originY);
  }

  /** Draw the in-progress polyline and the snapped cursor. */
  setPreview(points: Array<{ x: number; y: number }>, snapped: { x: number; y: number } | null): void {
    const px = 1 / this.view.scale;

    const g = this.previewLayer.clear();
    if (points.length >= 2) {
      g.moveTo(points[0]!.x, -points[0]!.y);
      for (let i = 1; i < points.length; i++) g.lineTo(points[i]!.x, -points[i]!.y);
      g.stroke({ color: this.colours.select, width: px * 2, alpha: 0.95 });
    }
    // Committed vertices, so a user can see what they have placed.
    for (let i = 0; i < points.length; i++) {
      g.circle(points[i]!.x, -points[i]!.y, px * 3).fill({
        color: this.colours.select,
        alpha: 0.9,
      });
    }

    const s = this.snapLayer.clear();
    if (snapped) {
      // A crosshair rather than a dot: it shows the exact point without hiding
      // the geometry underneath it, which matters when snapping to a corner.
      const r = px * 7;
      s.moveTo(snapped.x - r, -snapped.y).lineTo(snapped.x + r, -snapped.y);
      s.moveTo(snapped.x, -snapped.y - r).lineTo(snapped.x, -snapped.y + r);
      s.stroke({ color: this.colours.select, width: px, alpha: 0.85 });
    }
  }

  /** Highlight the selected element. Pass null to clear. */
  setSelection(points: Array<{ x: number; y: number }> | null, closed: boolean): void {
    const g = this.selectLayer.clear();
    if (!points || points.length < 2) return;
    const px = 1 / this.view.scale;

    g.moveTo(points[0]!.x, -points[0]!.y);
    for (let i = 1; i < points.length; i++) g.lineTo(points[i]!.x, -points[i]!.y);
    if (closed) g.closePath();
    // Drawn heavier than the geometry beneath so it reads as "this one",
    // without changing the colour of the thing being inspected.
    g.stroke({ color: this.colours.select, width: px * 4, alpha: 0.9 });

    for (const p of points) {
      g.circle(p.x, -p.y, px * 3.5).fill({ color: this.colours.select, alpha: 1 });
    }
  }

  clearPreview(): void {
    this.previewLayer.clear();
    this.snapLayer.clear();
  }

  /** Convert a client-space event position to world metres. */
  toWorld(clientX: number, clientY: number, host: HTMLElement): { x: number; y: number } {
    const rect = host.getBoundingClientRect();
    return this.screenToWorld(clientX - rect.left, clientY - rect.top);
  }

  /** Replace the venue geometry and fit it to the viewport. */
  setVenue(geometry: VenueGeometry): void {
    this.geometry = geometry;
    this.fit();
    this.redrawStatic();
  }

  /** Fit the venue to the viewport with a margin. */
  fit(): void {
    if (!this.geometry) return;
    const { minX, minY, maxX, maxY } = this.geometry.bounds;
    const w = Math.max(maxX - minX, 0.001);
    const h = Math.max(maxY - minY, 0.001);
    const sw = this.app.screen.width;
    const sh = this.app.screen.height;
    this.view.scale = Math.min(sw / w, sh / h) * 0.82;
    this.view.cx = (minX + maxX) / 2;
    this.view.cy = (minY + maxY) / 2;
    this.applyTransform();
  }

  /**
   * Upload agent positions.
   *
   * `xy` aliases wasm memory, so it is consumed immediately and never retained.
   * The particle pool grows but never shrinks: agents leave constantly during
   * an evacuation, and reallocating each tick would churn.
   */
  setAgents(xy: Float32Array, states: Uint8Array): void {
    const n = states.length;

    while (this.particles.length < n) {
      const p = new Particle({ texture: this.dot, anchorX: 0.5, anchorY: 0.5 });
      this.particles.push(p);
      this.agents.addParticle(p);
    }

    // World-space radius, in pixels, matched to a real shoulder width (~0.23 m).
    const r = (0.23 * this.view.scale) / 8;

    for (let i = 0; i < n; i++) {
      const p = this.particles[i]!;
      p.x = xy[i * 2]!;
      // Screen y grows downward; world y grows upward.
      p.y = -xy[i * 2 + 1]!;
      p.scaleX = r;
      p.scaleY = r;
      p.tint = states[i] === AgentState.Blocked ? this.colours.blocked : this.colours.walking;
      p.alpha = 1;
    }
    // Park surplus particles rather than destroying them.
    for (let i = n; i < this.particles.length; i++) {
      this.particles[i]!.alpha = 0;
    }
    this.agents.update();
  }

  clearAgents(): void {
    for (const p of this.particles) p.alpha = 0;
    this.agents.update();
  }

  private applyTransform(): void {
    const s = this.view.scale;
    this.world.scale.set(s);
    this.world.position.set(
      this.app.screen.width / 2 - this.view.cx * s,
      this.app.screen.height / 2 + this.view.cy * s,
    );
    // Line widths are specified in screen pixels, so they must be redrawn at
    // world scale or they thicken as you zoom in.
    this.redrawStatic();
  }

  /** Redraw everything that does not change per frame. */
  private redrawStatic(): void {
    const px = 1 / this.view.scale; // one screen pixel in world units
    this.drawGrid(px);
    this.drawFloor();
    this.drawWalls(px);
    this.drawDoors(px);
  }

  private drawGrid(px: number): void {
    const g = this.gridLayer.clear();
    if (!this.geometry) return;
    const { minX, minY, maxX, maxY } = this.geometry.bounds;
    const pad = 4;

    // A 1 m grid, with a heavier line every 5 m. Metres, because every
    // dimension in this product is metres and the grid is a ruler.
    for (let x = Math.floor(minX) - pad; x <= Math.ceil(maxX) + pad; x++) {
      const major = x % 5 === 0;
      g.moveTo(x, -(minY - pad)).lineTo(x, -(maxY + pad));
      g.stroke({ color: this.colours.grid, width: px * (major ? 1 : 0.5), alpha: major ? 0.7 : 0.35 });
    }
    for (let y = Math.floor(minY) - pad; y <= Math.ceil(maxY) + pad; y++) {
      const major = y % 5 === 0;
      g.moveTo(minX - pad, -y).lineTo(maxX + pad, -y);
      g.stroke({ color: this.colours.grid, width: px * (major ? 1 : 0.5), alpha: major ? 0.7 : 0.35 });
    }
  }

  private drawFloor(): void {
    const g = this.floorLayer.clear();
    const t = this.geometry?.triangles;
    if (!t) return;
    for (let i = 0; i + 5 < t.length; i += 6) {
      g.moveTo(t[i]!, -t[i + 1]!)
        .lineTo(t[i + 2]!, -t[i + 3]!)
        .lineTo(t[i + 4]!, -t[i + 5]!)
        .closePath();
    }
    g.fill({ color: this.colours.floor, alpha: 1 });
  }

  private drawWalls(px: number): void {
    const g = this.wallLayer.clear();
    const w = this.geometry?.walls;
    if (!w) return;
    for (let i = 0; i + 3 < w.length; i += 4) {
      g.moveTo(w[i]!, -w[i + 1]!).lineTo(w[i + 2]!, -w[i + 3]!);
    }
    g.stroke({ color: this.colours.wall, width: px * 2, alpha: 0.9 });
  }

  private drawDoors(px: number): void {
    const g = this.doorLayer.clear();
    const d = this.geometry?.doors;
    if (!d) return;
    for (let i = 0; i + 3 < d.length; i += 4) {
      g.moveTo(d[i]!, -d[i + 1]!).lineTo(d[i + 2]!, -d[i + 3]!);
    }
    // Doors are drawn heavier than walls: they are the thing the whole analysis
    // is about, and a reviewer should find them without looking.
    g.stroke({ color: this.colours.door, width: px * 4, alpha: 1 });
  }

  private installInput(host: HTMLElement): void {
    let pointerDown = false;
    let panning = false;
    let lastX = 0;
    let lastY = 0;
    let downX = 0;
    let downY = 0;

    // A click selects; a drag pans. Distinguished by whether the pointer moved
    // more than a few pixels — which is how every CAD tool behaves, and the
    // reason a select tool still needs pointer events rather than surrendering
    // them all to panning.
    const DRAG_THRESHOLD_PX = 4;

    host.addEventListener('dblclick', (e) => {
      this.onWorldPointer?.(this.toWorld(e.clientX, e.clientY, host), 'dblclick');
    });

    host.addEventListener('pointerdown', (e) => {
      pointerDown = true;
      panning = false;
      lastX = downX = e.clientX;
      lastY = downY = e.clientY;
      host.setPointerCapture(e.pointerId);
      // A drawing tool acts on press; select waits to see if this is a drag.
      if (this.drawing) {
        this.onWorldPointer?.(this.toWorld(e.clientX, e.clientY, host), 'down');
      }
    });

    host.addEventListener('pointerup', (e) => {
      if (pointerDown && !panning && !this.drawing) {
        // Moved less than the threshold, so it was a click, not a pan.
        this.onWorldPointer?.(this.toWorld(e.clientX, e.clientY, host), 'down');
      }
      pointerDown = false;
      panning = false;
      host.releasePointerCapture(e.pointerId);
    });

    host.addEventListener('pointermove', (e) => {
      // Tools always see movement, so previews track the cursor.
      this.onWorldPointer?.(this.toWorld(e.clientX, e.clientY, host), 'move');

      if (!pointerDown || this.drawing) return;

      if (!panning) {
        const moved = Math.hypot(e.clientX - downX, e.clientY - downY);
        if (moved < DRAG_THRESHOLD_PX) return;
        panning = true;
      }

      this.view.cx -= (e.clientX - lastX) / this.view.scale;
      this.view.cy += (e.clientY - lastY) / this.view.scale;
      lastX = e.clientX;
      lastY = e.clientY;
      this.applyTransform();
    });

    host.addEventListener(
      'wheel',
      (e) => {
        e.preventDefault();
        // Zoom about the cursor, so the point under the pointer stays put.
        const rect = host.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        const my = e.clientY - rect.top;
        const before = this.screenToWorld(mx, my);
        const factor = Math.exp(-e.deltaY * 0.0015);
        this.view.scale = Math.min(400, Math.max(2, this.view.scale * factor));
        const after = this.screenToWorld(mx, my);
        this.view.cx += before.x - after.x;
        this.view.cy += before.y - after.y;
        this.applyTransform();
      },
      { passive: false },
    );
  }

  private screenToWorld(sx: number, sy: number): { x: number; y: number } {
    const s = this.view.scale;
    return {
      x: (sx - this.app.screen.width / 2) / s + this.view.cx,
      y: -((sy - this.app.screen.height / 2) / s) + this.view.cy,
    };
  }
}
