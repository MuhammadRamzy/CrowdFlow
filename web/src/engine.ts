/**
 * Typed bridge to the Rust engine.
 *
 * `wasm-pack` is run with `--no-typescript` and the types are declared here
 * instead. That is deliberate: the generated `.d.ts` describes the marshalling,
 * not the contract, and hand-writing this layer is where the units, the
 * invariants and the caveats get to live. It is also the one file to read when
 * something at the boundary looks wrong.
 *
 * # Why this file is not in `src/engine/`
 *
 * It used to be, next to the generated bundle. `wasm-pack` writes its own
 * `.gitignore` containing `*` into its output directory, so this file was
 * invisible to git and was never committed — a fresh clone could not build, and
 * nothing noticed until the directory was deleted to test that very flow.
 * Generated output and authored source do not share a directory.
 *
 * Everything here is real. There is no mock path and no sample data — if the
 * engine has not produced a value, the UI shows nothing rather than a guess.
 */

import init, { CompiledVenue, Simulation, engine_version } from './engine/cf_wasm.js';

/** A compiler diagnostic, for the validation panel. */
export interface CompileWarning {
  /** Stable machine-readable code, e.g. `openingTooNarrow`. */
  code: string;
  /** Fatal warnings prevent simulation. */
  fatal: boolean;
  message: string;
}

/** Live counters from the running simulation. */
export interface SimStats {
  tick: number;
  /** Seconds. Derived from the tick count, so it does not drift. */
  time: number;
  active: number;
  exited: number;
  spawned: number;
  blocked: number;
  /** Metres. A persistently non-zero value means the contact solve is not converging. */
  maxOverlap: number;
  /** Agents recovered from outside the mesh. Should be zero. */
  escaped: number;
}

/** Geometry for the renderer. All coordinates are metres. */
export interface VenueGeometry {
  /** Flat `[x0,y0,x1,y1, ...]` wall segments. */
  walls: Float32Array;
  /** Flat `[ax,ay,bx,by,cx,cy, ...]` walkable triangles. */
  triangles: Float32Array;
  /** Flat `[ax,ay,bx,by, ...]` doorways. */
  doors: Float32Array;
  bounds: { minX: number; minY: number; maxX: number; maxY: number };
}

/** A snapshot of the crowd-density field, ready to upload as a texture. */
export interface DensityField {
  /** One byte per cell, row-major. 255 means `scale` persons/m². */
  bytes: Uint8Array;
  cols: number;
  rows: number;
  /** World-space origin of cell (0,0), metres. */
  originX: number;
  originY: number;
  /** Cell edge length, metres. */
  cell: number;
  /** Persons/m² represented by a byte value of 255. */
  scale: number;
}

/** Agent state discriminants, matching `cf_sim::world::AgentState`. */
export const AgentState = {
  Walking: 0,
  Queuing: 1,
  Dwelling: 2,
  Blocked: 3,
  Evacuating: 4,
  Exited: 5,
} as const;

let ready: Promise<void> | null = null;

/** Load and initialise the wasm module. Safe to call repeatedly. */
export function loadEngine(): Promise<void> {
  ready ??= init().then(() => undefined);
  return ready;
}

export function engineVersion(): string {
  return engine_version();
}

/**
 * A compiled venue: mesh, walls, doors and diagnostics.
 *
 * Compiling always succeeds structurally — problems come back as warnings
 * rather than exceptions, because a partial result plus diagnostics is more
 * useful to an editor than an error that loses everything. Only a malformed
 * *document* throws.
 */
export class Venue {
  private constructor(
    private readonly inner: CompiledVenue,
    readonly warnings: CompileWarning[],
    readonly geometry: VenueGeometry,
    readonly walkableArea: number,
    readonly simulable: boolean,
  ) {}

  static compile(documentJson: string): Venue {
    const inner = CompiledVenue.fromJson(documentJson);
    const b = inner.bounds(0);
    return new Venue(
      inner,
      inner.warnings() as CompileWarning[],
      {
        walls: inner.wallSegments(0),
        triangles: inner.walkableTriangles(0),
        doors: inner.doors(0),
        bounds: { minX: b[0]!, minY: b[1]!, maxX: b[2]!, maxY: b[3]! },
      },
      inner.walkableArea(),
      inner.isSimulable(),
    );
  }

  /** Warnings that prevent simulation. */
  get fatalWarnings(): CompileWarning[] {
    return this.warnings.filter((w) => w.fatal);
  }

  /** Start a simulation over this venue. Throws if the venue is not simulable. */
  simulate(seed: number): Run {
    if (!this.simulable) {
      throw new Error('venue has fatal warnings and cannot be simulated');
    }
    return new Run(new Simulation(this.inner, 0, seed));
  }

  /**
   * Start a run driven by an authored scenario.
   *
   * Nobody is placed up front. Agents arrive over time along each population's
   * arrival curve, through the doorways the scenario names, with body radii and
   * walking speeds drawn from its distributions. Read `notes` afterwards: it
   * lists anything in the document the engine could not act on.
   */
  runScenario(scenarioJson: string): Run {
    if (!this.simulable) {
      throw new Error('venue has fatal warnings and cannot be simulated');
    }
    return new Run(Simulation.fromScenario(this.inner, 0, scenarioJson));
  }

  free(): void {
    this.inner.free();
  }
}

/**
 * A running simulation.
 *
 * `positions()` returns a view that aliases wasm memory and is invalidated by
 * the next allocation inside wasm. Upload it to the GPU immediately; never hold
 * it across a tick.
 */
export class Run {
  constructor(private readonly inner: Simulation) {}

  /** Scatter agents across the walkable floor, each routed to its nearest exit. */
  spawn(count: number): number {
    return this.inner.spawnScattered(count);
  }

  /** Advance one physics tick (50 ms of simulated time). */
  step(): void {
    this.inner.step();
  }

  /** Advance `n` ticks. One boundary crossing instead of `n`. */
  stepMany(n: number): void {
    this.inner.stepMany(n);
  }

  /** Interleaved `[x, y, ...]` for every active agent, in metres. */
  positions(): Float32Array {
    return this.inner.positions();
  }

  /** One `AgentState` per active agent, ordered to match `positions()`. */
  states(): Uint8Array {
    return this.inner.states();
  }

  get active(): number {
    return this.inner.activeCount();
  }

  get exited(): number {
    return this.inner.exitedCount();
  }

  get spawned(): number {
    return this.inner.spawnedCount();
  }

  /** Simulated seconds elapsed. */
  get time(): number {
    return this.inner.time();
  }

  /**
   * Physics timestep in seconds. A scenario may specify its own, and the
   * playback clock has to agree with the engine about what a tick is worth.
   */
  get timestep(): number {
    return this.inner.timestep();
  }

  /**
   * Agents the scenario has authored but not yet admitted.
   *
   * Zero for a scattered placement. For a scenario run this is what makes an
   * empty venue at t=0 the *start* rather than the end: a run is over when this
   * and `active` are both zero.
   */
  get pending(): number {
    return this.inner.pendingCount();
  }

  /** Agents abandoned because their entrance never cleared. */
  get unplaced(): number {
    return this.inner.unplacedCount();
  }

  /** Everyone the scenario asks for, across all populations. */
  get scenarioTotal(): number {
    return this.inner.scenarioTotal();
  }

  /**
   * Everything in the scenario document this engine could not act on.
   *
   * Shown verbatim in the authoring panel. A field that is stored and
   * round-tripped but not simulated has to say so, or the control that edits it
   * is a lie.
   */
  scenarioNotes(): string[] {
    return this.inner.scenarioNotes() as string[];
  }

  stats(): SimStats {
    return this.inner.stats() as SimStats;
  }

  /**
   * The density field.
   *
   * `peak` selects the highest value each cell has reached rather than the
   * current one — which is what a reviewer wants, since the dangerous moment
   * matters even after the crowd has moved on.
   */
  density(peak: boolean): DensityField {
    const dims = this.inner.densityDims();
    const place = this.inner.densityPlacement();
    return {
      bytes: this.inner.densityBytes(peak),
      cols: dims[0]!,
      rows: dims[1]!,
      originX: place[0]!,
      originY: place[1]!,
      cell: place[2]!,
      scale: this.inner.densityScale(),
    };
  }

  /** Highest density reached anywhere during the run, persons/m². */
  get peakDensity(): number {
    return this.inner.peakDensity();
  }

  /** Floor area, m², that reached the crush threshold at any point. */
  get criticalArea(): number {
    return this.inner.criticalArea();
  }

  free(): void {
    this.inner.free();
  }
}
