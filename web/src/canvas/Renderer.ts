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

import { Application, Container, Graphics, Particle, ParticleContainer, Texture } from 'pixi.js';
import type { VenueGeometry } from '../engine/bridge';
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
  };

  async mount(host: HTMLElement): Promise<void> {
    this.colours = {
      grid: token('--line', 0x263140),
      floor: token('--sunken', 0x0f141b),
      wall: token('--chalk', 0xc9d3df),
      door: token('--normal', 0x3dd68c),
      walking: token('--select', 0x58c4e8),
      blocked: token('--supervise', 0xffb020),
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

    this.world.addChild(this.gridLayer, this.floorLayer, this.wallLayer, this.doorLayer, this.agents);
    this.app.stage.addChild(this.world);

    this.installInput(host);
    this.app.renderer.on('resize', () => this.redrawStatic());
  }

  destroy(): void {
    this.app?.destroy(true, { children: true });
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
    let dragging = false;
    let lastX = 0;
    let lastY = 0;

    host.addEventListener('pointerdown', (e) => {
      dragging = true;
      lastX = e.clientX;
      lastY = e.clientY;
      host.setPointerCapture(e.pointerId);
    });
    host.addEventListener('pointerup', (e) => {
      dragging = false;
      host.releasePointerCapture(e.pointerId);
    });
    host.addEventListener('pointermove', (e) => {
      if (!dragging) return;
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
